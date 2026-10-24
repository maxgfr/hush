//! Drops from the terminal: files sealed once, handed over peer to peer while both sides are
//! there, and otherwise waiting sealed on Blossom servers until the link is opened.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use hush_app::blossom::Blossom;
use hush_app::drops::{self, Progress, Source};
use hush_app::store::Contact;
use hush_app::{App, Cmd, Config, EndReason, Event, PathKind, util};
use hush_core::bundle::Bundle;
use hush_core::drop::{Opened, Recipient, Unlock, open_header};
use hush_core::identity::Identity;
use hush_core::link::{Link, Locator, Mode};
use tokio::sync::{mpsc, oneshot};

use crate::{human, print_qr, read_secret};

/// How long a receiver waits for the sender to answer before using the sealed copy.
const P2P_GRACE: Duration = Duration::from_secs(8);

pub struct SendOpts {
    pub files: Vec<PathBuf>,
    pub expires_min: u64,
    pub p2p_only: bool,
    pub many: bool,
    pub to: Vec<String>,
    pub to_self: bool,
    pub identity: Option<PathBuf>,
    pub wait: bool,
    pub passphrase: Option<String>,
}

fn bar(label: &'static str) -> Progress {
    Arc::new(move |done, total| {
        eprint!(
            "\r  {label}… {:>3}% ({} / {})   ",
            done * 100 / total.max(1),
            human(done),
            human(total)
        );
        let _ = std::io::stderr().flush();
    })
}

fn date(ms: u64) -> String {
    let left = (ms.saturating_sub(util::now_ms()) + 30_000) / 60_000;
    match left {
        m if m < 60 => format!("{m} min"),
        m if m < 48 * 60 => format!("{} h", (m + 30) / 60),
        m => format!("{} days", m / 1440),
    }
}

/// Our identity and contacts, from a backup made by the apps.
fn load_identity(path: Option<&PathBuf>) -> Result<(Identity, Vec<(Contact, Bundle)>)> {
    let Some(path) = path else {
        bail!("this needs your identity: --identity BACKUP (a backup made by the app)");
    };
    let data = std::fs::read(path).map_err(|e| anyhow!("reading {}: {e}", path.display()))?;
    let pass = match std::env::var("HUSH_BACKUP_PASSPHRASE") {
        Ok(p) => p,
        Err(_) => read_secret("backup passphrase")?,
    };
    let (me, contacts) =
        hush_app::store::read_backup(&data, &pass).map_err(|_| anyhow!("wrong passphrase or modified backup"))?;
    let me = me.ok_or_else(|| anyhow!("this backup holds no identity"))?;
    Ok((me, contacts))
}

fn summary(opened: &Opened) {
    let m = &opened.manifest;
    eprintln!(
        "  {} file{}, {}, available for {}{}",
        m.files.len(),
        if m.files.len() == 1 { "" } else { "s" },
        human(m.total()),
        date(m.expires_ms),
        if m.once { ", deleted once downloaded" } else { "" }
    );
    for f in m.files.iter().take(20) {
        eprintln!("    {}  ({})", f.path, human(f.size));
    }
    if m.files.len() > 20 {
        eprintln!("    … and {} more", m.files.len() - 20);
    }
}

fn saved_list(saved: &[PathBuf]) {
    eprintln!();
    for p in saved {
        eprintln!("  ✓ saved {} (SHA-256 verified)", p.display());
    }
}

/// Deletes a one-time drop once it arrived.
async fn burn(opened: &Opened, locator: &Locator, http: &Blossom) {
    if opened.manifest.once {
        if drops::delete(opened, locator, http).await {
            eprintln!("  one-time link: deleted from the servers");
        } else {
            eprintln!("  one-time link: asked the servers to delete it (not all confirmed)");
        }
    }
}

// ── send ────────────────────────────────────────────────────────────────────

pub async fn send(mut cfg: Config, opts: SendOpts) -> Result<()> {
    let sources = drops::collect(&opts.files)?;
    let total: u64 = sources.iter().map(|s| s.size).sum();
    eprintln!(
        "{} file{} ({})",
        sources.len(),
        if sources.len() == 1 { "" } else { "s" },
        human(total)
    );
    let expires_min = opts.expires_min.clamp(10, 7 * 24 * 60);
    let expires_ms = util::now_ms() + expires_min * 60_000;
    let servers = std::mem::take(&mut cfg.blossom);
    if !opts.to.is_empty() || opts.to_self {
        return send_sealed(&cfg.base, sources, &servers, &opts, expires_ms).await;
    }
    let pass = opts.passphrase.clone().filter(|p| !p.is_empty());
    let mut link = Link::generate(
        Mode::Drop,
        util::now_ms(),
        expires_min,
        pass.is_some(),
        &mut util::rng(),
    );
    let http = Blossom::new();
    let mut upload = None;
    if !opts.p2p_only {
        if servers.is_empty() {
            bail!("no blob server to keep a sealed copy: add --server HOST, or send with --p2p-only");
        }
        let root = link.drop_root(pass.as_deref()).map_err(|e| anyhow!("{e}"))?;
        let (src, once) = (sources.clone(), !opts.many);
        let prepared = tokio::task::spawn_blocking(move || {
            drops::prepare(
                src,
                &[Recipient::Secret(&root)],
                None,
                expires_ms,
                once,
                &bar("sealing"),
            )
        })
        .await??;
        eprintln!();
        let locator = drops::upload_header(&prepared, &http, &servers).await?;
        link.locator = Some(locator.clone());
        upload = Some((Arc::new(prepared), locator));
    }
    let (ev_tx, mut events) = mpsc::unbounded_channel();
    let app = App::start(cfg, ev_tx, None);
    let passphrase = pass.clone();
    let (chat, url) = app
        .ask(|reply| Cmd::OpenDrop {
            link,
            passphrase,
            reply,
        })
        .await?;
    eprintln!("\nSend this link (it expires in {}):\n", date(expires_ms));
    println!("{url}");
    print_qr(&url);
    if pass.is_some() {
        eprintln!("give the passphrase another way.");
    }

    let (done_tx, mut done_rx) = oneshot::channel::<Result<()>>();
    let mut uploading = None;
    if let Some((prepared, locator)) = upload.clone() {
        let http = http.clone();
        uploading = Some(tokio::spawn(async move {
            let r = drops::upload_segments(prepared, &http, &locator, bar("uploading the sealed copy")).await;
            let _ = done_tx.send(r);
        }));
    } else {
        drop(done_tx);
        eprintln!("peer to peer only: keep this running until it is delivered.");
    }
    let mut upload_done = upload.is_none();
    let mut peer_here = false;
    let mut delivered = 0usize;
    let finish = |app: &App| app.send(Cmd::End { chat });
    loop {
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    Event::PeerJoined { .. } => {
                        peer_here = true;
                        eprintln!("\n✓ the link was opened, and you are both online: sending peer to peer");
                        for s in &sources {
                            app.send(Cmd::SendFileAs { chat, path: s.path.clone(), rel: s.rel.clone() });
                        }
                    }
                    Event::PeerPath { path: PathKind::Relayed, .. } if upload.is_some() => {
                        eprintln!("  no direct path: they will download the sealed copy");
                    }
                    Event::FileDelivered { .. } => {
                        delivered += 1;
                        if delivered == sources.len() {
                            eprintln!("\n✓ delivered peer to peer (SHA-256 verified on their side)");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            finish(&app);
                            // Let the farewell leave before the process does.
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            // A one-time drop that arrived has no reason to stay on the servers.
                            if let (Some((prepared, locator)), false) = (&upload, opts.many) {
                                if let Some(t) = uploading.take() { t.abort(); }
                                let opened = open_header(&prepared.header, &Unlock::Key(prepared.key()), util::now_ms())?;
                                drops::delete(&opened, locator, &http).await;
                                eprintln!("  one-time link: the sealed copy was deleted");
                            }
                            return Ok(());
                        }
                    }
                    Event::ChatEnded { reason, .. } => {
                        peer_here = false;
                        if upload.is_none() {
                            match reason {
                                EndReason::Expired => bail!("the link expired"),
                                _ => bail!("the conversation ended before the files were delivered"),
                            }
                        }
                        if upload_done && !opts.wait { return Ok(()); }
                    }
                    _ => {}
                }
            }
            r = &mut done_rx, if !upload_done => {
                upload_done = true;
                match r.unwrap_or_else(|_| Err(anyhow!("the upload stopped"))) {
                    Ok(()) => {
                        let servers = upload.as_ref().map(|(_, l)| l.servers.join(", ")).unwrap_or_default();
                        eprintln!("\n✓ sealed copy uploaded to {servers}: the link works without you now (for {})", date(expires_ms));
                        if !peer_here && !opts.wait {
                            finish(&app);
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        eprintln!("\n✗ the sealed copy could not be uploaded: {e}");
                        eprintln!("  the link still works peer to peer while this keeps running.");
                        upload = None;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                finish(&app);
                if !upload_done {
                    eprintln!("\nstopped before the sealed copy was complete: the link will not work without you.");
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
                return Ok(());
            }
        }
    }
    Ok(())
}

async fn send_sealed(
    base: &str,
    sources: Vec<Source>,
    servers: &[String],
    opts: &SendOpts,
    expires_ms: u64,
) -> Result<()> {
    if servers.is_empty() {
        bail!("a drop sealed to someone waits on blob servers: add --server HOST");
    }
    let (me, contacts) = load_identity(opts.identity.as_ref())?;
    let epoch = Identity::spk_epoch(hush_core::utc_day(util::now_ms()));
    let mut keys: Vec<(String, Vec<u8>, u32)> = Vec::new();
    for want in &opts.to {
        let w = want.trim();
        let found = contacts
            .iter()
            .find(|(c, _)| c.id == w)
            .or_else(|| contacts.iter().find(|(c, _)| c.name.eq_ignore_ascii_case(w)));
        let Some((c, b)) = found else {
            bail!("{w} is not one of your contacts (by name or hush1… identifier)");
        };
        keys.push((c.name.clone(), b.spk.clone(), b.spk_epoch));
    }
    if opts.to_self {
        keys.push(("you".into(), me.spk(epoch).public(), epoch));
    }
    let recipients: Vec<Recipient<'_>> = keys
        .iter()
        .map(|(_, ek, epoch)| Recipient::Key { ek, epoch: *epoch })
        .collect();
    let once = !opts.many && keys.len() == 1 && !opts.to_self;
    let prepared = drops::prepare(sources, &recipients, Some(&me), expires_ms, once, &bar("sealing"))?;
    eprintln!();
    let http = Blossom::new();
    let locator = drops::upload_header(&prepared, &http, servers).await?;
    drops::upload_segments(Arc::new(prepared), &http, &locator, bar("uploading")).await?;
    let url = locator.sealed_url(base);
    let names: Vec<&str> = keys.iter().map(|(n, _, _)| n.as_str()).collect();
    eprintln!(
        "\n✓ sealed to {} and signed by you, on {} (for {})",
        names.join(", "),
        locator.servers.join(", "),
        date(expires_ms)
    );
    eprintln!("this link holds no key: only they can open it, so it can travel anywhere.\n");
    println!("{url}");
    print_qr(&url);
    Ok(())
}

// ── receive ─────────────────────────────────────────────────────────────────

/// A drop link: from the sender peer to peer if they are there, otherwise from the sealed copy.
pub async fn receive_link(mut cfg: Config, input: String, link: Link, pass: Option<String>) -> Result<()> {
    if link.is_expired(util::now_ms()) {
        eprintln!("this link has expired");
        std::process::exit(4);
    }
    let out = cfg.out_dir.clone();
    let http = Blossom::new();
    // The sealed copy, fetched meanwhile; the engine gets no servers of its own, so a drop is
    // never sealed again file by file.
    cfg.blossom.clear();
    let mut header = match &link.locator {
        Some(loc) => {
            let root = link.drop_root(pass.as_deref()).map_err(|e| anyhow!("{e}"))?;
            let (http, loc) = (http.clone(), loc.clone());
            Some(tokio::spawn(async move {
                let bytes = drops::fetch_header(&http, &loc).await?;
                open_header(&bytes, &Unlock::Secret(&root), util::now_ms()).map_err(|e| match e {
                    hush_core::Error::Decrypt => anyhow!("wrong passphrase, or a modified link"),
                    hush_core::Error::Expired => anyhow!("this drop has expired"),
                    e => anyhow!("{e}"),
                })
            }))
        }
        None => None,
    };
    let mut opened: Option<Opened> = None;
    let (ev_tx, mut events) = mpsc::unbounded_channel();
    let app = App::start(cfg, ev_tx, None);
    let chat = app
        .ask(|reply| Cmd::JoinLink {
            input,
            passphrase: pass,
            reply,
        })
        .await?;
    eprintln!("looking for the sender…");
    let grace = tokio::time::sleep(P2P_GRACE);
    tokio::pin!(grace);
    let mut joined = false;
    let mut saved = Vec::new();
    let progress = bar("receiving");
    let use_copy = loop {
        tokio::select! {
            h = async { header.as_mut().expect("guarded").await }, if header.is_some() => {
                header = None;
                match h.map_err(|e| anyhow!("{e}")).and_then(|r| r) {
                    Ok(o) => { summary(&o); opened = Some(o); }
                    Err(e) if !joined => {
                        app.send(Cmd::End { chat });
                        bail!("{e}");
                    }
                    Err(e) => eprintln!("  no sealed copy ({e}): receiving from the sender only"),
                }
            }
            ev = events.recv() => {
                let Some(ev) = ev else { break true };
                match ev {
                    Event::PeerJoined { .. } => {
                        joined = true;
                        eprintln!("✓ the sender is online: receiving peer to peer");
                    }
                    Event::PeerPath { path: PathKind::Relayed, .. } if link.locator.is_some() => {
                        eprintln!("  no direct path to the sender: using the sealed copy");
                        break true;
                    }
                    Event::FileOffered { peer: Some(_), id, .. } => app.send(Cmd::GetFile { chat, id }),
                    Event::FileProgress { done, total, .. } => progress(done, total),
                    Event::FileSaved { path, .. } => {
                        saved.push(path);
                        if opened.as_ref().is_some_and(|o| saved.len() == o.manifest.files.len()) {
                            break false;
                        }
                    }
                    Event::FileFailed { reason, .. } => {
                        eprintln!("\n  ✗ {reason}");
                        if link.locator.is_some() { break true; }
                    }
                    Event::ChatEnded { reason, .. } => match reason {
                        EndReason::PeerLeft if !saved.is_empty() => break false,
                        EndReason::Taken | EndReason::Failed | EndReason::PeerLeft | EndReason::Ended if link.locator.is_some() => break true,
                        EndReason::Taken => {
                            eprintln!("this link was already used by someone else.");
                            std::process::exit(3);
                        }
                        EndReason::Expired => { eprintln!("this link has expired"); std::process::exit(4); }
                        _ => bail!("the sender left before anything arrived"),
                    },
                    _ => {}
                }
            }
            () = &mut grace, if !joined && link.locator.is_some() => {
                eprintln!("  the sender is not online: using the sealed copy");
                break true;
            }
        }
    };
    app.send(Cmd::End { chat });
    // Let the last acknowledgements (`file-end`) and the farewell leave before going on.
    tokio::time::sleep(Duration::from_millis(500)).await;
    if let Some(h) = header.take() {
        match h.await.map_err(|e| anyhow!("{e}")).and_then(|r| r) {
            Ok(o) => {
                if use_copy {
                    summary(&o);
                }
                opened = Some(o);
            }
            Err(e) if use_copy => bail!("{e}"),
            Err(_) => {}
        }
    }
    let locator = link.locator.clone();
    if use_copy {
        let (Some(o), Some(loc)) = (&opened, &locator) else {
            bail!("the sender is not there, and this link has no sealed copy");
        };
        let files = drops::download(o, loc, &http, &out, bar("downloading")).await?;
        saved_list(&files);
    } else {
        eprintln!();
        for p in &saved {
            eprintln!("  ✓ saved {} (SHA-256 verified)", p.display());
        }
    }
    if let (Some(o), Some(loc)) = (&opened, &locator) {
        burn(o, loc, &http).await;
    }
    Ok(())
}

/// A drop sealed to us: our identity opens it.
pub async fn receive_sealed(locator: Locator, identity: Option<PathBuf>, out: PathBuf) -> Result<()> {
    let (me, contacts) = load_identity(identity.as_ref())?;
    let http = Blossom::new();
    let bytes = drops::fetch_header(&http, &locator).await?;
    let opened = open_header(&bytes, &Unlock::Identity(&me), util::now_ms()).map_err(|e| match e {
        hush_core::Error::Decrypt => anyhow!("this drop is not sealed to you"),
        hush_core::Error::Expired => anyhow!("this drop has expired"),
        e => anyhow!("{e}"),
    })?;
    match &opened.sender {
        Some(s) => {
            let name = contacts
                .iter()
                .find(|(c, _)| c.id == s.id())
                .map(|(c, _)| format!("{} ", c.name))
                .unwrap_or_default();
            eprintln!("from {name}{} (signature verified)", s.id());
        }
        None => eprintln!("unsigned"),
    }
    summary(&opened);
    let files = drops::download(&opened, &locator, &http, &out, bar("downloading")).await?;
    saved_list(&files);
    burn(&opened, &locator, &http).await;
    Ok(())
}

/// Deletes a drop from its servers (whoever can open it can delete it).
pub async fn remove(input: &str, identity: Option<PathBuf>, pass: Option<String>) -> Result<()> {
    let http = Blossom::new();
    let (opened, locator) = if let Ok(locator) = Locator::parse_sealed(input) {
        let (me, _) = load_identity(identity.as_ref())?;
        let bytes = drops::fetch_header(&http, &locator).await?;
        (
            open_header(&bytes, &Unlock::Identity(&me), 0).map_err(|_| anyhow!("this drop is not sealed to you"))?,
            locator,
        )
    } else {
        let link = Link::parse(input).map_err(util::link_error)?;
        let Some(locator) = link.locator.clone() else {
            bail!("this link has no sealed copy: nothing is stored anywhere");
        };
        let pass = match (link.passphrase, pass) {
            (true, None) => Some(read_secret("this link needs a passphrase")?),
            (_, p) => p,
        };
        let root = link.drop_root(pass.as_deref()).map_err(|e| anyhow!("{e}"))?;
        let bytes = drops::fetch_header(&http, &locator).await?;
        (
            open_header(&bytes, &Unlock::Secret(&root), 0).map_err(|_| anyhow!("wrong passphrase"))?,
            locator,
        )
    };
    if drops::delete(&opened, &locator, &http).await {
        eprintln!("✓ deleted from {}", locator.servers.join(", "));
    } else {
        eprintln!(
            "asked {} to delete it; not every server confirmed",
            locator.servers.join(", ")
        );
    }
    Ok(())
}
