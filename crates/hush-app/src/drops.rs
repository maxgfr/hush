//! Sealed drops between the disk and Blossom servers (see `hush_core::drop` for the format).
//!
//! Sending reads the files twice: once to learn the segment addresses and build the header, which
//! is published first, then again to upload the segments, each checked against the manifest before
//! it leaves (a file that changed in between would otherwise reuse a nonce on other bytes).
//! Receiving streams the segments into a hidden staging folder next to the destination, so an
//! interrupted download resumes from the last complete segment, and moves the files into place
//! only once every SHA-256 matches.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{StreamExt, TryStreamExt, stream};
use hush_core::drop::{
    DropFile, MAX_FILES, MAX_HEADER, MAX_TOTAL, Manifest, Opened, Recipient, SEGMENT, SEGMENT_CT, Sealer,
    segment_data_len, valid_path,
};
use hush_core::identity::Identity;
use hush_core::kdf::sha256;
use hush_core::link::Locator;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::blossom::{self, Blossom, COPIES};
use crate::text::{guess_mime, sanitize_file_name};
use crate::util::{now_ms, rng};

/// Segments uploaded or downloaded at once.
const PARALLEL: usize = 4;

/// Progress: `(done, total)` bytes.
pub type Progress = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// A file to send, and where it goes in the drop.
#[derive(Clone, Debug)]
pub struct Source {
    pub path: PathBuf,
    /// Relative path in the drop (`folder/sub/file.txt`).
    pub rel: String,
    pub size: u64,
}

/// The files to send: plain files by name, folders with everything under them (symbolic links and
/// special files are skipped, so a drop never leaves the folders it was given).
///
/// # Errors
/// For a missing path, a name that is not UTF-8, nothing to send, or more than a drop holds.
pub fn collect(paths: &[PathBuf]) -> Result<Vec<Source>> {
    let mut out = Vec::new();
    for p in paths {
        let meta = fs::symlink_metadata(p).with_context(|| format!("{}", p.display()))?;
        let name = p
            .canonicalize()
            .ok()
            .and_then(|c| c.file_name().map(|n| n.to_owned()))
            .and_then(|n| n.to_str().map(str::to_string))
            .ok_or_else(|| anyhow!("{}: the name is not valid UTF-8", p.display()))?;
        if meta.is_dir() {
            walk(p, &name, &mut out)?;
        } else if meta.is_file() {
            out.push(Source {
                path: p.clone(),
                rel: name,
                size: meta.len(),
            });
        } else {
            bail!("{} is not a file or a folder", p.display());
        }
        if out.len() > MAX_FILES {
            bail!("a drop holds at most {MAX_FILES} files");
        }
    }
    if out.is_empty() {
        bail!("nothing to send: the folders are empty");
    }
    if out.iter().map(|s| s.size).sum::<u64>() > MAX_TOTAL {
        bail!("a drop holds at most 4 GB");
    }
    if let Some(bad) = out.iter().find(|s| !valid_path(&s.rel)) {
        bail!("{}: cannot be sent under this name", bad.path.display());
    }
    Ok(out)
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<Source>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for e in entries {
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            bail!("{}: the name is not valid UTF-8", e.path().display());
        };
        let ft = e.file_type()?;
        let rel = format!("{rel}/{name}");
        if ft.is_dir() {
            walk(&e.path(), &rel, out)?;
        } else if ft.is_file() {
            out.push(Source {
                path: e.path(),
                size: e.metadata()?.len(),
                rel,
            });
        }
        if out.len() > MAX_FILES {
            bail!("a drop holds at most {MAX_FILES} files");
        }
    }
    Ok(())
}

/// Reads the files one after the other as one stream, hashing each file on the way.
struct StreamReader<'a> {
    sources: &'a [Source],
    idx: usize,
    file: Option<File>,
    left: u64,
    hasher: Sha256,
    hashes: Vec<[u8; 32]>,
}

impl<'a> StreamReader<'a> {
    fn new(sources: &'a [Source]) -> Self {
        Self {
            sources,
            idx: 0,
            file: None,
            left: 0,
            hasher: Sha256::new(),
            hashes: Vec::new(),
        }
    }

    /// Fills `buf` exactly.
    fn read(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            if self.file.is_none() {
                self.next_file()?;
                continue;
            }
            let n = usize::try_from(self.left.min(buf.len() as u64)).expect("bounded");
            let f = self.file.as_mut().expect("open");
            f.read_exact(&mut buf[..n]).map_err(|_| {
                anyhow!(
                    "{} changed while it was being sent",
                    self.sources[self.idx].path.display()
                )
            })?;
            self.hasher.update(&buf[..n]);
            self.left -= n as u64;
            buf = &mut buf[n..];
            if self.left == 0 {
                self.finish_file();
            }
        }
        Ok(())
    }

    fn next_file(&mut self) -> Result<()> {
        let s = self
            .sources
            .get(self.idx)
            .ok_or_else(|| anyhow!("a file shrank while it was being sent"))?;
        let f = File::open(&s.path).with_context(|| format!("{}", s.path.display()))?;
        if f.metadata()?.len() != s.size {
            bail!("{} changed while it was being sent", s.path.display());
        }
        self.left = s.size;
        self.file = Some(f);
        if s.size == 0 {
            self.finish_file();
        }
        Ok(())
    }

    fn finish_file(&mut self) {
        self.file = None;
        self.hashes.push(std::mem::take(&mut self.hasher).finalize().into());
        self.idx += 1;
    }

    /// Opens the empty files left at the end, so they get their hash too.
    fn finish(mut self) -> Result<Vec<[u8; 32]>> {
        while self.idx < self.sources.len() {
            if self.file.is_none() {
                self.next_file()?;
            } else {
                bail!("a file grew while it was being sent");
            }
        }
        Ok(self.hashes)
    }
}

/// A drop ready to upload.
pub struct Prepared {
    sealer: Sealer,
    pub sources: Vec<Source>,
    pub manifest: Manifest,
    pub header: Vec<u8>,
}

impl Prepared {
    /// The file key, for a peer that receives the drop inside a session.
    #[must_use]
    pub fn key(&self) -> &[u8; 32] {
        self.sealer.key()
    }

    #[must_use]
    pub fn total(&self) -> u64 {
        self.sealer.total()
    }
}

/// Seals the files once to build the header (slow: reads everything; call it off the async
/// runtime). `progress` sees the bytes read.
///
/// # Errors
/// For an unreadable file or one that changes meanwhile.
pub fn prepare(
    sources: Vec<Source>,
    recipients: &[Recipient<'_>],
    signer: Option<&Identity>,
    expires_ms: u64,
    once: bool,
    progress: &Progress,
) -> Result<Prepared> {
    let total: u64 = sources.iter().map(|s| s.size).sum();
    let sealer = Sealer::new(total, &mut rng()).map_err(|_| anyhow!("a drop holds at most 4 GB"))?;
    let mut segments = Vec::with_capacity(sealer.segment_count() as usize);
    let mut reader = StreamReader::new(&sources);
    let mut buf = vec![0u8; SEGMENT];
    let mut done = 0u64;
    for i in 0..sealer.segment_count() {
        let len = segment_data_len(total, i);
        reader.read(&mut buf[..len])?;
        segments.push(sha256(&[&sealer.seal_segment(i, &buf[..len]).expect("right length")]));
        done += len as u64;
        progress(done, total);
    }
    let hashes = reader.finish()?;
    let now = now_ms();
    let manifest = Manifest {
        files: sources
            .iter()
            .zip(hashes)
            .map(|(s, h)| DropFile {
                path: s.rel.clone(),
                size: s.size,
                mime: guess_mime(&s.rel).to_string(),
                sha256: h,
            })
            .collect(),
        created_ms: now,
        expires_ms: expires_ms.max(now + 60_000),
        once,
        segments,
        delete_key: blossom::new_key(),
    };
    let header = sealer
        .header(&manifest, recipients, signer, &mut rng())
        .map_err(|e| anyhow!("cannot seal this drop: {e}"))?;
    Ok(Prepared {
        sealer,
        sources,
        manifest,
        header,
    })
}

/// Uploads the header to [`COPIES`] of `servers`: the locator names those that took it.
///
/// # Errors
/// When no server takes it.
pub async fn upload_header(p: &Prepared, http: &Blossom, servers: &[String]) -> Result<Locator> {
    let sk = blossom::signing_key(&p.manifest.delete_key)?;
    let hosts = http.put_any(servers, COPIES, &p.header, &sk).await?;
    Locator::new(sha256(&[&p.header]), hosts).map_err(|_| anyhow!("invalid server name"))
}

/// Uploads every segment to the locator's servers (each to all of them; one copy is enough).
///
/// # Errors
/// For a file that changed since [`prepare`], or a segment no server took.
pub async fn upload_segments(p: Arc<Prepared>, http: &Blossom, locator: &Locator, progress: Progress) -> Result<()> {
    let sk = blossom::signing_key(&p.manifest.delete_key)?;
    let (tx, rx) = mpsc::channel::<Result<(u32, Vec<u8>)>>(PARALLEL);
    let producer = Arc::clone(&p);
    tokio::task::spawn_blocking(move || {
        let p = producer;
        let total = p.total();
        let mut reader = StreamReader::new(&p.sources);
        let mut buf = vec![0u8; SEGMENT];
        for i in 0..p.sealer.segment_count() {
            let len = segment_data_len(total, i);
            let sealed = reader.read(&mut buf[..len]).and_then(|()| {
                let ct = p.sealer.seal_segment(i, &buf[..len]).expect("right length");
                if sha256(&[&ct]) == p.manifest.segments[i as usize] {
                    Ok((i, ct))
                } else {
                    Err(anyhow!("a file changed while it was being sent: start again"))
                }
            });
            let stop = sealed.is_err();
            if tx.blocking_send(sealed).is_err() || stop {
                return;
            }
        }
    });
    let total = p.total();
    let done = std::sync::atomic::AtomicU64::new(0);
    tokio_stream(rx)
        .map(|r| {
            let (sk, hosts, done, progress) = (&sk, &locator.servers, &done, &progress);
            async move {
                let (i, ct) = r?;
                let mut ok = false;
                let mut last = String::new();
                for h in hosts {
                    match http.put(h, &ct, sk).await {
                        Ok(()) => ok = true,
                        Err(e) => last = e.to_string(),
                    }
                }
                if !ok {
                    bail!("segment {i}: {last}");
                }
                let n = segment_data_len(total, i) as u64;
                let d = done.fetch_add(n, std::sync::atomic::Ordering::Relaxed) + n;
                progress(d, total);
                Ok(())
            }
        })
        .buffer_unordered(PARALLEL)
        .try_collect::<()>()
        .await
}

fn tokio_stream<T>(mut rx: mpsc::Receiver<T>) -> impl futures_util::Stream<Item = T> {
    stream::poll_fn(move |cx| rx.poll_recv(cx))
}

/// Fetches a drop's header.
///
/// # Errors
/// When no server has it.
pub async fn fetch_header(http: &Blossom, locator: &Locator) -> Result<Vec<u8>> {
    http.get(&locator.servers, &locator.header, MAX_HEADER)
        .await
        .map_err(|e| anyhow!("the drop is not on its servers any more (deleted or expired): {e}"))
}

/// Deletes the header and every segment from the locator's servers. Best effort: returns whether
/// every server confirmed every blob.
pub async fn delete(opened: &Opened, locator: &Locator, http: &Blossom) -> bool {
    let Ok(sk) = blossom::signing_key(&opened.manifest.delete_key) else {
        return false;
    };
    let want = locator.servers.len();
    let mut all = true;
    for sha in opened.manifest.segments.iter().chain([&locator.header]) {
        all &= http.delete(&locator.servers, sha, &sk).await == want;
    }
    all
}

/// The staging folder of a download: hidden, next to the destination, one per drop.
fn staging(out_dir: &Path, locator: &Locator) -> PathBuf {
    out_dir.join(format!(".hush-drop-{}", &hex::encode(locator.header)[..16]))
}

/// Downloads a drop into `out_dir`, resuming an interrupted download. Returns where each file
/// was saved (folders are recreated, never overwriting anything).
///
/// # Errors
/// When a segment cannot be fetched or does not authenticate, a file's SHA-256 does not match, or
/// the disk fails.
pub async fn download(
    opened: &Opened,
    locator: &Locator,
    http: &Blossom,
    out_dir: &Path,
    progress: Progress,
) -> Result<Vec<PathBuf>> {
    let m = &opened.manifest;
    let total = m.total();
    let stage = staging(out_dir, locator);
    fs::create_dir_all(&stage)?;
    let start = resume_point(m, &stage)?;
    let mut writer = StreamWriter::open(m, &stage, start)?;
    progress(start, total);
    let first = u32::try_from(start / SEGMENT as u64).expect("bounded");
    let fetches = stream::iter(first..opened.segment_count())
        .map(|i| async move {
            let ct = http
                .get(&locator.servers, &m.segments[i as usize], SEGMENT_CT)
                .await
                .map_err(|e| anyhow!("segment {i} is missing: {e}"))?;
            let pt = opened
                .open_segment(i, &ct)
                .map_err(|_| anyhow!("segment {i} does not authenticate"))?;
            Ok::<_, anyhow::Error>(pt)
        })
        .buffered(PARALLEL);
    futures_util::pin_mut!(fetches);
    let mut done = start;
    while let Some(pt) = fetches.next().await {
        let pt = pt?;
        writer.write(&pt)?;
        done += pt.len() as u64;
        progress(done, total);
    }
    let hashes = writer.finish()?;
    for (f, h) in m.files.iter().zip(&hashes) {
        if *h != f.sha256 {
            bail!("{}: SHA-256 mismatch, nothing was saved", f.path);
        }
    }
    let mut saved = Vec::with_capacity(m.files.len());
    for (i, f) in m.files.iter().enumerate() {
        saved.push(place(&stage.join(i.to_string()), out_dir, &f.path)?);
    }
    let _ = fs::remove_dir_all(&stage);
    Ok(saved)
}

/// Where to resume: the start of the first segment not completely on disk. Truncates what lies
/// past it.
fn resume_point(m: &Manifest, stage: &Path) -> Result<u64> {
    let mut contiguous = 0u64;
    for (i, f) in m.files.iter().enumerate() {
        let len = fs::metadata(stage.join(i.to_string())).map_or(0, |md| md.len());
        contiguous += len.min(f.size);
        if len < f.size {
            break;
        }
    }
    let at = contiguous / SEGMENT as u64 * SEGMENT as u64;
    for (i, (f, offset)) in m.files.iter().zip(m.offsets()).enumerate() {
        let path = stage.join(i.to_string());
        let keep = at.saturating_sub(offset).min(f.size);
        if path.exists() {
            OpenOptions::new().write(true).open(&path)?.set_len(keep)?;
        }
    }
    Ok(at)
}

/// Writes the stream into one staging file per drop file, hashing as it goes.
struct StreamWriter<'a> {
    m: &'a Manifest,
    stage: &'a Path,
    idx: usize,
    file: Option<File>,
    left: u64,
    hasher: Sha256,
    hashes: Vec<[u8; 32]>,
}

impl<'a> StreamWriter<'a> {
    /// Opens at stream position `at`, re-hashing what is already there.
    fn open(m: &'a Manifest, stage: &'a Path, at: u64) -> Result<Self> {
        let mut w = Self {
            m,
            stage,
            idx: 0,
            file: None,
            left: 0,
            hasher: Sha256::new(),
            hashes: Vec::new(),
        };
        for (f, offset) in m.files.iter().zip(m.offsets()) {
            let path = stage.join(w.idx.to_string());
            let end = offset + f.size;
            if end < at || end == at && (f.size > 0 || path.exists()) {
                // Complete: hash it again.
                w.rehash(&path)?;
                w.finish_file();
            } else if offset < at {
                // Partly there: hash what is, append the rest.
                w.rehash(&path)?;
                w.left = end - at;
                w.file = Some(OpenOptions::new().append(true).open(&path)?);
                break;
            } else {
                break;
            }
        }
        Ok(w)
    }

    fn rehash(&mut self, path: &Path) -> Result<()> {
        let mut r = File::open(path)?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            self.hasher.update(&buf[..n]);
        }
    }

    fn write(&mut self, mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            if self.file.is_none() {
                self.next_file()?;
                continue;
            }
            let n = usize::try_from(self.left.min(data.len() as u64)).expect("bounded");
            self.file.as_mut().expect("open").write_all(&data[..n])?;
            self.hasher.update(&data[..n]);
            self.left -= n as u64;
            data = &data[n..];
            if self.left == 0 {
                self.finish_file();
            }
        }
        Ok(())
    }

    fn next_file(&mut self) -> Result<()> {
        let f = self
            .m
            .files
            .get(self.idx)
            .ok_or_else(|| anyhow!("more data than the manifest says"))?;
        let path = self.stage.join(self.idx.to_string());
        self.file = Some(OpenOptions::new().create(true).write(true).truncate(true).open(path)?);
        self.left = f.size;
        if f.size == 0 {
            self.finish_file();
        }
        Ok(())
    }

    fn finish_file(&mut self) {
        if let Some(f) = self.file.take() {
            let _ = f.sync_all();
        }
        self.hashes.push(std::mem::take(&mut self.hasher).finalize().into());
        self.idx += 1;
    }

    fn finish(mut self) -> Result<Vec<[u8; 32]>> {
        while self.idx < self.m.files.len() {
            if self.file.is_some() {
                bail!("the drop ended early");
            }
            self.next_file()?;
        }
        Ok(self.hashes)
    }
}

/// Creates the folders of `rel` under `out_dir` (each name made safe, never through a symbolic
/// link); returns the folder and the safe file name.
///
/// # Errors
/// When something that is not a folder is in the way, or the disk fails.
pub fn folder_for(out_dir: &Path, rel: &str) -> Result<(PathBuf, String)> {
    let mut parts: Vec<String> = rel.split('/').map(sanitize_file_name).collect();
    let name = parts.pop().unwrap_or_else(|| "file".into());
    let mut dir = out_dir.to_path_buf();
    for d in parts {
        dir.push(d);
        match fs::symlink_metadata(&dir) {
            Ok(md) if md.is_dir() => {}
            Ok(_) => bail!("{}: in the way of a folder that was sent", dir.display()),
            Err(_) => fs::create_dir(&dir)?,
        }
    }
    Ok((dir, name))
}

/// Moves a downloaded file to `out_dir/rel`, recreating its folders and never replacing an
/// existing file: `name (1)`, `name (2)`…
fn place(src: &Path, out_dir: &Path, rel: &str) -> Result<PathBuf> {
    let (dir, name) = folder_for(out_dir, rel)?;
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    for n in 0..10_000 {
        let target = if n == 0 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem} ({n}){ext}"))
        };
        // A hard link never replaces an existing path; a copy for file systems without them.
        match fs::hard_link(src, &target) {
            Ok(()) => {
                let _ = fs::remove_file(src);
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                let (mut out, path) = crate::text::create_unique(&dir, &name)?;
                std::io::copy(&mut File::open(src)?, &mut out)?;
                out.sync_all()?;
                let _ = fs::remove_file(src);
                return Ok(path);
            }
        }
    }
    bail!("no free file name in {}", dir.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, len: usize) -> Vec<u8> {
        let data: Vec<u8> = (0..len).map(|i| (i * 7 % 253) as u8).collect();
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, &data).unwrap();
        data
    }

    #[test]
    fn collects_folders_in_order_and_skips_links() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "photos/b.jpg", 3);
        write(tmp.path(), "photos/a/x.txt", 1);
        write(tmp.path(), "single.pdf", 2);
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", tmp.path().join("photos/link")).unwrap();
        let got = collect(&[tmp.path().join("photos"), tmp.path().join("single.pdf")]).unwrap();
        let rels: Vec<_> = got.iter().map(|s| s.rel.as_str()).collect();
        assert_eq!(rels, ["photos/a/x.txt", "photos/b.jpg", "single.pdf"]);
        fs::create_dir(tmp.path().join("empty")).unwrap();
        assert!(collect(&[tmp.path().join("empty")]).is_err());
        assert!(collect(&[tmp.path().join("missing")]).is_err());
    }

    #[test]
    fn stream_writer_resumes_at_a_segment_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = |path: &str, size: u64| DropFile {
            path: path.into(),
            size,
            mime: String::new(),
            sha256: [0; 32],
        };
        let seg = SEGMENT as u64;
        let m = Manifest {
            files: vec![mk("a", seg + 5), mk("e", 0), mk("b", seg)],
            created_ms: 0,
            expires_ms: 1,
            once: false,
            segments: vec![[0; 32]; 3],
            delete_key: [0; 32],
        };
        let stream: Vec<u8> = (0..(2 * seg + 5) as usize).map(|i| (i % 251) as u8).collect();
        // Nothing yet: from the start.
        assert_eq!(resume_point(&m, tmp.path()).unwrap(), 0);
        // All of "a" and part of "b": resume at the second segment, "a" cut back.
        fs::write(tmp.path().join("0"), &stream[..(seg + 5) as usize]).unwrap();
        fs::write(tmp.path().join("2"), &stream[(seg + 5) as usize..(seg + 100) as usize]).unwrap();
        let at = resume_point(&m, tmp.path()).unwrap();
        assert_eq!(at, seg);
        assert_eq!(fs::metadata(tmp.path().join("0")).unwrap().len(), seg);
        assert_eq!(fs::metadata(tmp.path().join("2")).unwrap().len(), 0);
        let mut w = StreamWriter::open(&m, tmp.path(), at).unwrap();
        w.write(&stream[seg as usize..]).unwrap();
        let hashes = w.finish().unwrap();
        assert_eq!(
            hashes[0],
            <[u8; 32]>::from(Sha256::digest(&stream[..(seg + 5) as usize]))
        );
        assert_eq!(hashes[1], <[u8; 32]>::from(Sha256::digest(b"")));
        assert_eq!(
            hashes[2],
            <[u8; 32]>::from(Sha256::digest(&stream[(seg + 5) as usize..]))
        );
    }

    #[test]
    fn place_never_overwrites_nor_follows_links() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        fs::create_dir(&out).unwrap();
        for i in 0..2 {
            let src = tmp.path().join(format!("s{i}"));
            fs::write(&src, format!("v{i}")).unwrap();
            let p = place(&src, &out, "dir/file.txt").unwrap();
            assert!(p.starts_with(&out));
            assert!(!src.exists());
        }
        assert_eq!(fs::read_to_string(out.join("dir/file.txt")).unwrap(), "v0");
        assert_eq!(fs::read_to_string(out.join("dir/file (1).txt")).unwrap(), "v1");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path(), out.join("evil")).unwrap();
            let src = tmp.path().join("s3");
            fs::write(&src, "x").unwrap();
            assert!(place(&src, &out, "evil/x").is_err());
        }
    }
}
