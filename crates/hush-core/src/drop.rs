//! Sealed drops: files encrypted once, stored as opaque blobs anywhere (Blossom servers in
//! practice), and opened by whoever holds the link, by the identities they were sealed to, or by a
//! peer that received the file key inside an end-to-end session.
//!
//! ```text
//! header      = "HUSHDR" || v(1) || drop_id(16) || n(1) || stanza{n} || m_len(u32) || manifest_ct
//! stanza      = 0x01 || XChaCha(Expand(drop_root, "hush/v2/drop/wrap-link" || drop_id), aad, K)
//!             | 0x02 || epoch(u32) || ct(1120) || XChaCha(wrap_kem, aad, K)
//! wrap_kem    = Expand(Extract("hush/v2/drop/kem", ss || ct || ek), "hush/v2/drop/wrap-kem" || drop_id)
//! aad         = "HUSHDR" || v || drop_id
//! manifest_ct = XChaCha(Expand(K, "hush/v2/drop/manifest"), aad = header up to m_len, pad(manifest))
//! segment_i   = ChaCha20-Poly1305(Expand(K, "hush/v2/drop/payload"), nonce(i, last), aad = drop_id, pt_i)
//! nonce(i, l) = 0 0 0 l || u64 i                                             (STREAM, as in age)
//! ```
//!
//! The files are one stream, cut into 4 MiB segments. Every segment but the last is full and the
//! last is padded to 4 MiB (or, for a drop of a single segment, to its Padmé length), so the stored
//! blobs say little more than the number of segments. Each segment opens on its own, which allows
//! streaming, resuming and fetching segments from different servers; the nonce binds its index and
//! whether it is the last, so segments cannot be reordered, dropped or the stream truncated.
//!
//! The manifest lists the files, the SHA-256 of every stored segment (the blobs' addresses), the
//! expiry, and a throwaway key that may delete the blobs. It can carry the sender's hybrid
//! signature, which also covers the stanzas: a recipient learns who sent it, and nobody holding the
//! file key can re-address it as if it came from the sender.
//!
//! Segments are sealed deterministically (fixed key, counter nonce), so a host can seal a stream
//! once to learn the segment addresses, publish the header first, then seal again while uploading.
//! The host must check each segment's SHA-256 against the manifest before uploading it: a file that
//! changed in between would otherwise reuse a nonce on different bytes.

use zeroize::Zeroizing;

use crate::aead::{TAG_LEN, XNONCE_LEN, open, seal, xopen, xseal};
use crate::identity::{ED_PK_LEN, Identity, MLDSA_PK_LEN, PublicIdentity, SIG_LEN};
use crate::kdf::{expand32, extract, sha256};
use crate::kem::{CT_LEN, KemSecret, encapsulate};
use crate::pad::{pad, padme, unpad};
use crate::{Error, Result, Rng};

const MAGIC: &[u8; 6] = b"HUSHDR";
const DROP_VERSION: u8 = 2;
const ID_LEN: usize = 16;
const PREFIX_LEN: usize = MAGIC.len() + 1 + ID_LEN;
const WRAPPED_LEN: usize = XNONCE_LEN + 32 + TAG_LEN;
const STANZA_LINK: u8 = 1;
const STANZA_KEM: u8 = 2;

/// Plaintext bytes per segment.
pub const SEGMENT: usize = 4 * 1024 * 1024;
/// Stored bytes per full segment.
pub const SEGMENT_CT: usize = SEGMENT + TAG_LEN;
/// Largest drop: the same bound as a file sent peer to peer.
pub const MAX_TOTAL: u64 = 4 * 1024 * 1024 * 1024;
/// Most files in one drop.
pub const MAX_FILES: usize = 4096;
/// Most recipients (stanzas) in one header.
pub const MAX_STANZAS: usize = 16;
/// Longest relative path, in bytes.
pub const MAX_PATH: usize = 1024;
/// Largest header a host should fetch.
pub const MAX_HEADER: usize = 8 * 1024 * 1024;
const MAX_MIME: usize = 127;
/// A single-segment drop is padded to at least this much.
const MIN_SEGMENT: usize = 1024;

/// A file in a drop. Files follow each other in the stream, in this order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropFile {
    /// Relative path with `/` separators (see [`valid_path`]).
    pub path: String,
    pub size: u64,
    pub mime: String,
    /// SHA-256 of the file's plaintext.
    pub sha256: [u8; 32],
}

/// What a drop holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub files: Vec<DropFile>,
    pub created_ms: u64,
    pub expires_ms: u64,
    /// The sender asked that the first recipient delete the blobs once downloaded.
    pub once: bool,
    /// SHA-256 of every stored segment, in order: the blobs' addresses.
    pub segments: Vec<[u8; 32]>,
    /// A throwaway secp256k1 secret allowed to delete the blobs (Blossom BUD-02).
    pub delete_key: [u8; 32],
}

impl Manifest {
    /// Total length of the stream.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Where each file starts in the stream.
    #[must_use]
    pub fn offsets(&self) -> Vec<u64> {
        self.files
            .iter()
            .scan(0u64, |at, f| {
                let start = *at;
                *at += f.size;
                Some(start)
            })
            .collect()
    }
}

/// Whether a path is safe to recreate under a download folder: relative, `/`-separated, no empty,
/// `.` or `..` component, no backslash, NUL or control character, no zero-width or bidirectional
/// override (which could show `report\u{202e}fdp.exe` as `reportexe.pdf`), at most [`MAX_PATH`]
/// bytes.
#[must_use]
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH
        && !path
            .chars()
            .any(|c| c.is_control() || c == '\\' || crate::bundle::SPOOFING.contains(&c))
        && path.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// Number of segments for a stream of `total` bytes (an empty stream still has one).
#[must_use]
pub fn segment_count(total: u64) -> u32 {
    u32::try_from(total.div_ceil(SEGMENT as u64).max(1)).unwrap_or(u32::MAX)
}

/// How many stream bytes segment `index` carries.
#[must_use]
pub fn segment_data_len(total: u64, index: u32) -> usize {
    let start = u64::from(index) * SEGMENT as u64;
    usize::try_from(total.saturating_sub(start).min(SEGMENT as u64)).expect("at most a segment")
}

fn segment_padded_len(total: u64, index: u32) -> usize {
    if segment_count(total) == 1 {
        padme(segment_data_len(total, index).max(MIN_SEGMENT)).min(SEGMENT)
    } else {
        SEGMENT
    }
}

fn segment_nonce(index: u32, last: bool) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[3] = u8::from(last);
    nonce[4..].copy_from_slice(&u64::from(index).to_be_bytes());
    nonce
}

/// Someone a drop is sealed to.
pub enum Recipient<'a> {
    /// Whoever holds a link: its [`crate::link::Link::drop_root`].
    Secret(&'a [u8; 32]),
    /// An identity: its signed prekey (from its bundle) and the prekey's epoch.
    Key { ek: &'a [u8], epoch: u32 },
}

/// How to open a header.
pub enum Unlock<'a> {
    /// A link's [`crate::link::Link::drop_root`].
    Secret(&'a [u8; 32]),
    /// Our identity: tries every identity stanza.
    Identity(&'a Identity),
    /// The file key itself, received inside a session.
    Key(&'a [u8; 32]),
}

fn prefix(drop_id: &[u8; ID_LEN]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PREFIX_LEN);
    p.extend_from_slice(MAGIC);
    p.push(DROP_VERSION);
    p.extend_from_slice(drop_id);
    p
}

fn wrap_link(root: &[u8; 32], drop_id: &[u8; ID_LEN]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(expand32(root, b"hush/v2/drop/wrap-link", &[drop_id]))
}

fn wrap_kem(ss: &[u8; 32], ct: &[u8], ek: &[u8], drop_id: &[u8; ID_LEN]) -> Zeroizing<[u8; 32]> {
    let prk = Zeroizing::new(extract(b"hush/v2/drop/kem", &[ss.as_slice(), ct, ek].concat()));
    Zeroizing::new(expand32(&prk, b"hush/v2/drop/wrap-kem", &[drop_id]))
}

/// Seals one drop: its key, identifier and stream length.
pub struct Sealer {
    key: Zeroizing<[u8; 32]>,
    payload: Zeroizing<[u8; 32]>,
    drop_id: [u8; ID_LEN],
    total: u64,
}

impl Sealer {
    /// A new drop of `total` stream bytes, with a fresh file key.
    ///
    /// # Errors
    /// [`Error::InvalidParameter`] above [`MAX_TOTAL`].
    pub fn new<R: Rng + ?Sized>(total: u64, rng: &mut R) -> Result<Self> {
        if total > MAX_TOTAL {
            return Err(Error::InvalidParameter);
        }
        let key: [u8; 32] = crate::random(rng);
        Ok(Self {
            payload: Zeroizing::new(expand32(&key, b"hush/v2/drop/payload", &[])),
            key: Zeroizing::new(key),
            drop_id: crate::random(rng),
            total,
        })
    }

    /// The file key, to hand to a peer inside an end-to-end session ([`Unlock::Key`]).
    #[must_use]
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    #[must_use]
    pub fn segment_count(&self) -> u32 {
        segment_count(self.total)
    }

    /// Seals segment `index`, whose stream bytes are `data` (exactly
    /// [`segment_data_len`]`(total, index)` of them). Deterministic: the same input gives the same
    /// output.
    ///
    /// # Errors
    /// [`Error::InvalidParameter`] for an index past the end or a wrong length.
    pub fn seal_segment(&self, index: u32, data: &[u8]) -> Result<Vec<u8>> {
        let count = self.segment_count();
        if index >= count || data.len() != segment_data_len(self.total, index) {
            return Err(Error::InvalidParameter);
        }
        let mut pt = Zeroizing::new(Vec::with_capacity(segment_padded_len(self.total, index)));
        pt.extend_from_slice(data);
        pt.resize(segment_padded_len(self.total, index), 0);
        Ok(seal(
            &self.payload,
            &segment_nonce(index, index + 1 == count),
            &self.drop_id,
            &pt,
        ))
    }

    /// Builds the header. `manifest.segments` must hold the SHA-256 of every sealed segment, and
    /// the files must add up to the stream length. With `signer`, the manifest carries the
    /// sender's signature.
    ///
    /// # Errors
    /// [`Error::InvalidParameter`] for an inconsistent manifest, too many recipients or an invalid
    /// path; [`Error::Malformed`] for an invalid recipient key.
    pub fn header<R: Rng + ?Sized>(
        &self,
        manifest: &Manifest,
        recipients: &[Recipient<'_>],
        signer: Option<&Identity>,
        rng: &mut R,
    ) -> Result<Vec<u8>> {
        check_manifest(manifest)?;
        if manifest.total() != self.total
            || manifest.segments.len() != self.segment_count() as usize
            || recipients.len() > MAX_STANZAS
        {
            return Err(Error::InvalidParameter);
        }
        let aad = prefix(&self.drop_id);
        let mut out = aad.clone();
        out.push(u8::try_from(recipients.len()).expect("bounded"));
        let stanzas_at = out.len();
        for r in recipients {
            match r {
                Recipient::Secret(root) => {
                    out.push(STANZA_LINK);
                    out.extend_from_slice(&xseal(&wrap_link(root, &self.drop_id), &aad, &*self.key, rng));
                }
                Recipient::Key { ek, epoch } => {
                    let (ct, ss) = encapsulate(ek, rng)?;
                    out.push(STANZA_KEM);
                    out.extend_from_slice(&epoch.to_be_bytes());
                    out.extend_from_slice(&ct);
                    out.extend_from_slice(&xseal(&wrap_kem(&ss, &ct, ek, &self.drop_id), &aad, &*self.key, rng));
                }
            }
        }
        let stanzas_hash = sha256(&[&out[stanzas_at..]]);
        let mut body = encode_manifest(manifest);
        match signer {
            None => body.push(0),
            Some(me) => {
                let sig = me.sign(b"drop", &signed(&self.drop_id, &stanzas_hash, &body), rng);
                body.push(1);
                body.extend_from_slice(me.public().ed_pk());
                body.extend_from_slice(me.public().mldsa_pk());
                body.extend_from_slice(&sig);
            }
        }
        let manifest_key = Zeroizing::new(expand32(&self.key, b"hush/v2/drop/manifest", &[]));
        let ct = xseal(&manifest_key, &out, &pad(&body), rng);
        out.extend_from_slice(&u32::try_from(ct.len()).expect("bounded manifest").to_be_bytes());
        out.extend_from_slice(&ct);
        Ok(out)
    }
}

fn signed(drop_id: &[u8; ID_LEN], stanzas_hash: &[u8; 32], body: &[u8]) -> Vec<u8> {
    [drop_id.as_slice(), stanzas_hash, &sha256(&[body])].concat()
}

fn check_manifest(m: &Manifest) -> Result<()> {
    let total = m
        .files
        .iter()
        .try_fold(0u64, |acc, f| acc.checked_add(f.size))
        .ok_or(Error::InvalidParameter)?;
    if m.files.is_empty()
        || m.files.len() > MAX_FILES
        || total > MAX_TOTAL
        || m.segments.len() != segment_count(total) as usize
        || m.expires_ms < m.created_ms
        || m.files.iter().any(|f| !valid_path(&f.path) || f.mime.len() > MAX_MIME)
    {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}

fn encode_manifest(m: &Manifest) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&m.created_ms.to_be_bytes());
    b.extend_from_slice(&m.expires_ms.to_be_bytes());
    b.push(u8::from(m.once));
    b.extend_from_slice(&m.delete_key);
    b.extend_from_slice(&u32::try_from(m.segments.len()).expect("bounded").to_be_bytes());
    for s in &m.segments {
        b.extend_from_slice(s);
    }
    b.extend_from_slice(&u32::try_from(m.files.len()).expect("bounded").to_be_bytes());
    for f in &m.files {
        b.extend_from_slice(&u16::try_from(f.path.len()).expect("bounded").to_be_bytes());
        b.extend_from_slice(f.path.as_bytes());
        b.extend_from_slice(&f.size.to_be_bytes());
        b.push(u8::try_from(f.mime.len()).expect("bounded"));
        b.extend_from_slice(f.mime.as_bytes());
        b.extend_from_slice(&f.sha256);
    }
    b
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self
            .b
            .get(self.at..self.at.checked_add(n).ok_or(Error::Malformed)?)
            .ok_or(Error::Malformed)?;
        self.at += n;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| Error::Malformed)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn str(&mut self, n: usize) -> Result<String> {
        core::str::from_utf8(self.take(n)?)
            .map(str::to_string)
            .map_err(|_| Error::Malformed)
    }
}

fn decode_manifest(r: &mut Reader<'_>) -> Result<Manifest> {
    let created_ms = r.u64()?;
    let expires_ms = r.u64()?;
    let once = match r.u8()? {
        0 => false,
        1 => true,
        _ => return Err(Error::Malformed),
    };
    let delete_key = r.array()?;
    let n_segments = r.u32()? as usize;
    if n_segments > segment_count(MAX_TOTAL) as usize {
        return Err(Error::Malformed);
    }
    let segments = (0..n_segments).map(|_| r.array()).collect::<Result<Vec<_>>>()?;
    let n_files = r.u32()? as usize;
    if n_files > MAX_FILES {
        return Err(Error::Malformed);
    }
    let mut files = Vec::with_capacity(n_files);
    for _ in 0..n_files {
        let path_len = usize::from(r.u16()?);
        let path = r.str(path_len)?;
        let size = r.u64()?;
        let mime_len = usize::from(r.u8()?);
        let mime = r.str(mime_len)?;
        files.push(DropFile {
            path,
            size,
            mime,
            sha256: r.array()?,
        });
    }
    let m = Manifest {
        files,
        created_ms,
        expires_ms,
        once,
        segments,
        delete_key,
    };
    check_manifest(&m).map_err(|_| Error::Malformed)?;
    Ok(m)
}

/// An opened header: the manifest, who signed it, and the key to open the segments.
pub struct Opened {
    key: Zeroizing<[u8; 32]>,
    payload: Zeroizing<[u8; 32]>,
    drop_id: [u8; ID_LEN],
    pub manifest: Manifest,
    /// The sender, when the manifest is signed (the signature has been verified).
    pub sender: Option<PublicIdentity>,
}

impl core::fmt::Debug for Opened {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Opened")
            .field("manifest", &self.manifest)
            .field("sender", &self.sender)
            .finish_non_exhaustive()
    }
}

/// Opens a header.
///
/// # Errors
/// [`Error::Malformed`] when it does not parse, [`Error::Decrypt`] when no stanza opens with
/// `unlock` (wrong link, passphrase or identity) or the manifest was tampered with,
/// [`Error::BadSignature`] for a forged signature, [`Error::Expired`] once past its expiry.
pub fn open_header(header: &[u8], unlock: &Unlock<'_>, now_ms: u64) -> Result<Opened> {
    if header.len() > MAX_HEADER {
        return Err(Error::Malformed);
    }
    let mut r = Reader { b: header, at: 0 };
    if r.take(MAGIC.len())? != MAGIC || r.u8()? != DROP_VERSION {
        return Err(Error::Malformed);
    }
    let drop_id: [u8; ID_LEN] = r.array()?;
    let aad = prefix(&drop_id);
    let n = usize::from(r.u8()?);
    if n > MAX_STANZAS {
        return Err(Error::Malformed);
    }
    let stanzas_at = r.at;
    let mut key: Option<Zeroizing<[u8; 32]>> = match unlock {
        Unlock::Key(k) => Some(Zeroizing::new(**k)),
        _ => None,
    };
    for _ in 0..n {
        let opened = match r.u8()? {
            STANZA_LINK => {
                let wrapped = r.take(WRAPPED_LEN)?;
                match unlock {
                    Unlock::Secret(root) => xopen(&wrap_link(root, &drop_id), &aad, wrapped).ok(),
                    _ => None,
                }
            }
            STANZA_KEM => {
                let epoch = r.u32()?;
                let ct = r.take(CT_LEN)?;
                let wrapped = r.take(WRAPPED_LEN)?;
                match unlock {
                    Unlock::Identity(me) => {
                        let spk: KemSecret = me.spk(epoch);
                        spk.decapsulate(ct)
                            .ok()
                            .and_then(|ss| xopen(&wrap_kem(&ss, ct, &spk.public(), &drop_id), &aad, wrapped).ok())
                    }
                    _ => None,
                }
            }
            _ => return Err(Error::Malformed),
        };
        if key.is_none()
            && let Some(k) = opened
        {
            key = Some(Zeroizing::new(k.try_into().map_err(|_| Error::Malformed)?));
        }
    }
    let stanzas_hash = sha256(&[&header[stanzas_at..r.at]]);
    let manifest_aad = &header[..r.at];
    let m_len = r.u32()? as usize;
    let ct = r.take(m_len)?;
    if r.at != header.len() {
        return Err(Error::Malformed);
    }
    let key = key.ok_or(Error::Decrypt)?;
    let manifest_key = Zeroizing::new(expand32(&key, b"hush/v2/drop/manifest", &[]));
    let padded = Zeroizing::new(xopen(&manifest_key, manifest_aad, ct)?);
    let body = unpad(&padded)?;
    let mut mr = Reader { b: body, at: 0 };
    let manifest = decode_manifest(&mut mr)?;
    let signed_len = mr.at;
    let sender = match mr.u8()? {
        0 => None,
        1 => {
            let ed_pk = mr.take(ED_PK_LEN)?;
            let mldsa_pk = mr.take(MLDSA_PK_LEN)?;
            let sig = mr.take(SIG_LEN)?;
            let who = PublicIdentity::from_keys(ed_pk, mldsa_pk)?;
            who.verify(b"drop", &signed(&drop_id, &stanzas_hash, &body[..signed_len]), sig)?;
            Some(who)
        }
        _ => return Err(Error::Malformed),
    };
    if mr.at != body.len() {
        return Err(Error::Malformed);
    }
    if now_ms >= manifest.expires_ms {
        return Err(Error::Expired);
    }
    Ok(Opened {
        payload: Zeroizing::new(expand32(&key, b"hush/v2/drop/payload", &[])),
        key,
        drop_id,
        manifest,
        sender,
    })
}

impl Opened {
    /// The file key (to forward the drop inside a session).
    #[must_use]
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    #[must_use]
    pub fn segment_count(&self) -> u32 {
        segment_count(self.manifest.total())
    }

    /// Checks a stored segment's address, then opens it: returns its stream bytes.
    ///
    /// # Errors
    /// [`Error::InvalidParameter`] for an index past the end, [`Error::Decrypt`] when the blob is
    /// not the one the manifest names or does not authenticate, [`Error::Malformed`] when its
    /// padding is not zero.
    pub fn open_segment(&self, index: u32, ct: &[u8]) -> Result<Vec<u8>> {
        let count = self.segment_count();
        let expected = self
            .manifest
            .segments
            .get(index as usize)
            .ok_or(Error::InvalidParameter)?;
        if sha256(&[ct]) != *expected {
            return Err(Error::Decrypt);
        }
        let total = self.manifest.total();
        let mut pt = open(
            &self.payload,
            &segment_nonce(index, index + 1 == count),
            &self.drop_id,
            ct,
        )?;
        let data_len = segment_data_len(total, index);
        if pt.len() != segment_padded_len(total, index) || pt[data_len..].iter().any(|&b| b != 0) {
            return Err(Error::Malformed);
        }
        pt.truncate(data_len);
        Ok(pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{Link, Mode};
    use crate::vault::KdfParams;

    const CHEAP: KdfParams = KdfParams { m_kib: 64, t: 1, p: 1 };
    const NOW: u64 = 1_750_000_000_000;

    fn stream(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    /// Seals `files` (name, bytes) and returns the header and the stored segments.
    fn make(
        files: &[(&str, &[u8])],
        recipients: &[Recipient<'_>],
        signer: Option<&Identity>,
    ) -> (Vec<u8>, Vec<Vec<u8>>, [u8; 32]) {
        let mut rng = crate::test_rng::rng();
        let data: Vec<u8> = files.iter().flat_map(|(_, b)| b.iter().copied()).collect();
        let sealer = Sealer::new(data.len() as u64, &mut rng).unwrap();
        let segments: Vec<Vec<u8>> = (0..sealer.segment_count())
            .map(|i| {
                let start = i as usize * SEGMENT;
                let len = segment_data_len(data.len() as u64, i);
                sealer.seal_segment(i, &data[start..start + len]).unwrap()
            })
            .collect();
        let manifest = Manifest {
            files: files
                .iter()
                .map(|(name, b)| DropFile {
                    path: (*name).to_string(),
                    size: b.len() as u64,
                    mime: "text/plain".into(),
                    sha256: sha256(&[b]),
                })
                .collect(),
            created_ms: NOW,
            expires_ms: NOW + 60_000,
            once: true,
            segments: segments.iter().map(|s| sha256(&[s])).collect(),
            delete_key: [5; 32],
        };
        let header = sealer.header(&manifest, recipients, signer, &mut rng).unwrap();
        (header, segments, *sealer.key())
    }

    fn reassemble(opened: &Opened, segments: &[Vec<u8>]) -> Vec<u8> {
        segments
            .iter()
            .enumerate()
            .flat_map(|(i, s)| opened.open_segment(i as u32, s).unwrap())
            .collect()
    }

    #[test]
    fn link_round_trip_across_segments() {
        let mut rng = crate::test_rng::rng();
        let link = Link::generate(Mode::Drop, NOW, 60, true, &mut rng);
        let root = link.drop_root_with(Some("pass"), CHEAP).unwrap();
        let a = stream(SEGMENT + 1000);
        let b = stream(SEGMENT * 2 - 2000);
        let (header, segments, _) = make(&[("a.bin", &a), ("dir/b.bin", &b)], &[Recipient::Secret(&root)], None);
        assert_eq!(segments.len(), 3);
        assert!(
            segments.iter().all(|s| s.len() == SEGMENT_CT),
            "every blob has the same size"
        );

        let opened = open_header(&header, &Unlock::Secret(&root), NOW).unwrap();
        assert_eq!(opened.manifest.files.len(), 2);
        assert_eq!(opened.manifest.files[1].path, "dir/b.bin");
        assert_eq!(opened.manifest.offsets(), vec![0, a.len() as u64]);
        assert!(opened.sender.is_none());
        assert!(opened.manifest.once);
        assert_eq!(reassemble(&opened, &segments), [a, b].concat());

        let wrong = link.drop_root_with(Some("Pass"), CHEAP).unwrap();
        assert_eq!(
            open_header(&header, &Unlock::Secret(&wrong), NOW).unwrap_err(),
            Error::Decrypt
        );
        assert_eq!(
            open_header(&header, &Unlock::Secret(&root), NOW + 60_000).unwrap_err(),
            Error::Expired
        );
    }

    #[test]
    fn small_drops_are_padded() {
        let (_, short, _) = make(&[("a", b"hi")], &[Recipient::Secret(&[1; 32])], None);
        let (_, other, _) = make(&[("a", &stream(900))], &[Recipient::Secret(&[1; 32])], None);
        assert_eq!(short[0].len(), other[0].len());
        assert_eq!(short[0].len(), MIN_SEGMENT + TAG_LEN);
        let (header, empty, _) = make(&[("empty", b"")], &[Recipient::Secret(&[1; 32])], None);
        let opened = open_header(&header, &Unlock::Secret(&[1; 32]), NOW).unwrap();
        assert_eq!(opened.open_segment(0, &empty[0]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn identity_recipients_and_signature() {
        let mut rng = crate::test_rng::rng();
        let alice = Identity::generate(&mut rng);
        let bob = Identity::generate(&mut rng);
        let carol = Identity::generate(&mut rng);
        let epoch = Identity::spk_epoch(crate::utc_day(NOW));
        let bob_ek = bob.spk(epoch).public();
        let alice_ek = alice.spk(epoch - 1).public();
        let (header, segments, _) = make(
            &[("note.txt", b"for bob and me")],
            &[
                Recipient::Key { ek: &bob_ek, epoch },
                Recipient::Key {
                    ek: &alice_ek,
                    epoch: epoch - 1,
                },
            ],
            Some(&alice),
        );
        for who in [&bob, &alice] {
            let opened = open_header(&header, &Unlock::Identity(who), NOW).unwrap();
            assert_eq!(opened.sender.as_ref(), Some(alice.public()));
            assert_eq!(reassemble(&opened, &segments), b"for bob and me");
        }
        assert_eq!(
            open_header(&header, &Unlock::Identity(&carol), NOW).unwrap_err(),
            Error::Decrypt
        );
        assert_eq!(
            open_header(&header, &Unlock::Secret(&[0; 32]), NOW).unwrap_err(),
            Error::Decrypt
        );
    }

    #[test]
    fn session_key_opens_without_stanzas() {
        let (header, segments, key) = make(&[("x", b"in a chat")], &[], None);
        let opened = open_header(&header, &Unlock::Key(&key), NOW).unwrap();
        assert_eq!(reassemble(&opened, &segments), b"in a chat");
        assert_eq!(opened.key(), &key);
        assert_eq!(
            open_header(&header, &Unlock::Key(&[0; 32]), NOW).unwrap_err(),
            Error::Decrypt
        );
    }

    #[test]
    fn tampering_is_detected() {
        let a = stream(SEGMENT + 10);
        let (header, segments, key) = make(&[("a", &a)], &[Recipient::Secret(&[3; 32])], None);
        let opened = open_header(&header, &Unlock::Key(&key), NOW).unwrap();
        // Every byte of the header is authenticated.
        for i in [0, 7, PREFIX_LEN, PREFIX_LEN + 5, header.len() - 1] {
            let mut h = header.clone();
            h[i] ^= 1;
            assert!(open_header(&h, &Unlock::Secret(&[3; 32]), NOW).is_err(), "byte {i}");
        }
        assert!(open_header(&header[..header.len() - 1], &Unlock::Key(&key), NOW).is_err());
        assert!(open_header(&[header.as_slice(), &[0]].concat(), &Unlock::Key(&key), NOW).is_err());
        // A flipped bit, swapped segments, or a segment under another index.
        let mut s0 = segments[0].clone();
        s0[10] ^= 1;
        assert_eq!(opened.open_segment(0, &s0).unwrap_err(), Error::Decrypt);
        assert_eq!(opened.open_segment(0, &segments[1]).unwrap_err(), Error::Decrypt);
        assert_eq!(
            opened.open_segment(2, &segments[0]).unwrap_err(),
            Error::InvalidParameter
        );
    }

    #[test]
    fn truncation_and_reordering_fail_even_with_a_forged_manifest() {
        // Someone holding the key rewrites the manifest to list only the first segment: the stream
        // ends early, but segment 0 was sealed as "not last", so it does not open as the last.
        let mut rng = crate::test_rng::rng();
        let a = stream(SEGMENT + 10);
        let sealer = Sealer::new(a.len() as u64, &mut rng).unwrap();
        let s0 = sealer.seal_segment(0, &a[..SEGMENT]).unwrap();
        let forged = Sealer {
            key: Zeroizing::new(*sealer.key()),
            payload: Zeroizing::new(*sealer.payload),
            drop_id: sealer.drop_id,
            total: SEGMENT as u64,
        };
        let manifest = Manifest {
            files: vec![DropFile {
                path: "a".into(),
                size: SEGMENT as u64,
                mime: String::new(),
                sha256: [0; 32],
            }],
            created_ms: NOW,
            expires_ms: NOW + 1,
            once: false,
            segments: vec![sha256(&[&s0])],
            delete_key: [0; 32],
        };
        let header = forged.header(&manifest, &[], None, &mut rng).unwrap();
        let opened = open_header(&header, &Unlock::Key(sealer.key()), NOW).unwrap();
        assert_eq!(opened.open_segment(0, &s0).unwrap_err(), Error::Decrypt);
    }

    /// What servers store shows nothing of the files: no name, no pattern of the content, and
    /// bytes indistinguishable from random even for a file of a single repeated byte.
    #[test]
    fn nothing_in_clear() {
        let name = "rapport-confidentiel-2026.pdf";
        let zeros = vec![0u8; SEGMENT + 4096];
        let pattern: Vec<u8> = b"ATTACK AT DAWN ".iter().copied().cycle().take(100_000).collect();
        let (header, segments, _) = make(
            &[(name, &zeros), ("notes.txt", &pattern)],
            &[Recipient::Secret(&[7; 32])],
            None,
        );
        let stored: Vec<&[u8]> = std::iter::once(header.as_slice())
            .chain(segments.iter().map(Vec::as_slice))
            .collect();
        for blob in &stored {
            for needle in [name.as_bytes(), b"notes.txt", b"ATTACK AT DAWN", b"text/plain"] {
                assert!(!blob.windows(needle.len()).any(|w| w == needle));
            }
        }
        for seg in &segments {
            // No 8-byte block repeats (ECB-like leaks), and every byte value is about as frequent
            // as the others: a chi-squared test over 256 values (99.99% bound: about 375).
            let mut seen = std::collections::HashSet::new();
            assert!(seg.chunks_exact(8).all(|c| seen.insert(c)), "a repeated block");
            let mut counts = [0f64; 256];
            for &b in seg.iter() {
                counts[usize::from(b)] += 1.0;
            }
            let expected = seg.len() as f64 / 256.0;
            let chi2: f64 = counts.iter().map(|c| (c - expected).powi(2) / expected).sum();
            assert!(chi2 < 375.0, "bytes are not uniform: chi² = {chi2}");
        }
        // Two drops of the same files share no byte pattern: fresh keys each time.
        let (_, again, _) = make(
            &[(name, &zeros), ("notes.txt", &pattern)],
            &[Recipient::Secret(&[7; 32])],
            None,
        );
        assert_ne!(again[0][..64], segments[0][..64]);
    }

    #[test]
    fn deterministic_segments() {
        let mut rng = crate::test_rng::rng();
        let sealer = Sealer::new(100, &mut rng).unwrap();
        let data = stream(100);
        assert_eq!(
            sealer.seal_segment(0, &data).unwrap(),
            sealer.seal_segment(0, &data).unwrap()
        );
        assert!(sealer.seal_segment(0, &data[..99]).is_err());
        assert!(sealer.seal_segment(1, &[]).is_err());
        assert!(Sealer::new(MAX_TOTAL + 1, &mut rng).is_err());
    }

    #[test]
    fn manifests_are_checked() {
        for bad in [
            "",
            "/etc/passwd",
            "a/../b",
            "./a",
            "a//b",
            "a\\b",
            "a\u{0}b",
            "a/\n",
            "x\u{202e}fdp.exe",
            "a\u{200b}b",
        ] {
            assert!(!valid_path(bad), "{bad:?}");
        }
        for good in ["a", "dir/sub/file.txt", "été .pdf", "..a", "a.."] {
            assert!(valid_path(good), "{good:?}");
        }
        let mut rng = crate::test_rng::rng();
        let sealer = Sealer::new(2, &mut rng).unwrap();
        let s = sealer.seal_segment(0, b"ab").unwrap();
        let mut m = Manifest {
            files: vec![DropFile {
                path: "../x".into(),
                size: 2,
                mime: String::new(),
                sha256: [0; 32],
            }],
            created_ms: NOW,
            expires_ms: NOW + 1,
            once: false,
            segments: vec![sha256(&[&s])],
            delete_key: [0; 32],
        };
        assert!(sealer.header(&m, &[], None, &mut rng).is_err());
        m.files[0].path = "x".into();
        m.files[0].size = 3;
        assert!(sealer.header(&m, &[], None, &mut rng).is_err());
        m.files[0].size = 2;
        assert!(sealer.header(&m, &[], None, &mut rng).is_ok());
        let too_many: Vec<Recipient<'_>> = (0..=MAX_STANZAS).map(|_| Recipient::Secret(&[0; 32])).collect();
        assert!(sealer.header(&m, &too_many, None, &mut rng).is_err());
    }
}
