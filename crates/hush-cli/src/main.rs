//! `hush`: post-quantum, end-to-end encrypted chat and file transfer from the terminal, in the same
//! conversations as the web app (https://maxgfr.github.io/hush/).

mod room;
mod rtc;
mod signal;
mod util;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use room::{Command as RoomCmd, EndReason, Event, Room, RoomOptions};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use webrtc::peer_connection::RTCIceServer;

const DEFAULT_BASE: &str = "https://maxgfr.github.io/hush/";
const DEFAULT_STUN: [&str; 2] = ["stun:stun.cloudflare.com:3478", "stun:stun.l.google.com:19302"];

#[derive(Parser)]
#[command(
    name = "hush",
    version,
    about = "Post-quantum, end-to-end encrypted chat and file transfer. Send a link, talk, end it: nothing is left."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print a new link (offline: nothing is sent anywhere).
    New {
        /// A group link (anyone holding it can join) instead of a one-to-one link.
        #[arg(long)]
        group: bool,
        /// Minutes before the link expires.
        #[arg(long, default_value_t = 60)]
        expires: u64,
        /// The link will require a passphrase, given through another channel.
        #[arg(long)]
        with_passphrase: bool,
        #[arg(long, default_value = DEFAULT_BASE)]
        base: String,
    },
    /// Chat: lines you type are sent, messages are printed. Without a link, creates one.
    Chat {
        link: Option<String>,
        /// Create a group link instead of a one-to-one link.
        #[arg(long)]
        group: bool,
        /// Minutes before a created link expires.
        #[arg(long, default_value_t = 60)]
        expires: u64,
        /// Directory where received files are saved.
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
        #[command(flatten)]
        net: NetArgs,
    },
    /// Send files to whoever opens the link, then exit.
    Send {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Join this link instead of creating one.
        #[arg(long)]
        link: Option<String>,
        #[arg(long, default_value_t = 60)]
        expires: u64,
        #[command(flatten)]
        net: NetArgs,
    },
    /// Receive every file sent in a conversation, until the sender ends it.
    Receive {
        link: String,
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
        #[command(flatten)]
        net: NetArgs,
    },
    /// Encrypted backups made by the web app (or this CLI).
    Backup {
        #[command(subcommand)]
        action: BackupCmd,
    },
}

#[derive(Subcommand)]
enum BackupCmd {
    /// Decrypt a backup and list what it holds (identifier, contacts, kept history).
    Inspect {
        file: PathBuf,
        #[arg(long, env = "HUSH_BACKUP_PASSPHRASE")]
        passphrase: Option<String>,
    },
    /// Re-encrypt a backup under a new passphrase.
    Rekey {
        file: PathBuf,
        /// Where to write the re-encrypted backup.
        #[arg(long)]
        out: PathBuf,
        #[arg(long, env = "HUSH_BACKUP_PASSPHRASE")]
        passphrase: Option<String>,
        #[arg(long, env = "HUSH_NEW_BACKUP_PASSPHRASE")]
        new_passphrase: Option<String>,
    },
}

#[derive(Args, Clone)]
struct NetArgs {
    /// Your display name.
    #[arg(long, default_value = "")]
    name: String,
    /// Link passphrase: required to open a protected link, makes a created link protected.
    #[arg(long, env = "HUSH_PASSPHRASE")]
    passphrase: Option<String>,
    /// Disappearing-message timer for what you send, in seconds.
    #[arg(long, default_value_t = 0)]
    ttl: u64,
    /// Nostr relay (repeatable). Defaults to the web app's relays.
    #[arg(long = "relay")]
    relays: Vec<String>,
    /// WebTorrent tracker (repeatable). Defaults to the web app's trackers.
    #[arg(long = "tracker")]
    trackers: Vec<String>,
    #[arg(long)]
    no_nostr: bool,
    #[arg(long)]
    no_torrent: bool,
    /// STUN server (repeatable). Defaults to Cloudflare and Google.
    #[arg(long = "stun")]
    stun: Vec<String>,
    /// No STUN server (local network only).
    #[arg(long)]
    no_stun: bool,
    /// TURN server URL, e.g. turns:turn.example.com:443.
    #[arg(long)]
    turn: Option<String>,
    #[arg(long, default_value = "")]
    turn_user: String,
    #[arg(long, default_value = "", env = "HUSH_TURN_CREDENTIAL")]
    turn_credential: String,
    /// Only use TURN: the other side never learns your IP address.
    #[arg(long)]
    relay_only: bool,
    /// No relay or tracker at all: the handshake is carried by hand as codes (/code to paste).
    #[arg(long)]
    offline: bool,
    /// Base URL of the web app, for printed links.
    #[arg(long, default_value = DEFAULT_BASE)]
    base: String,
}

impl NetArgs {
    fn options(&self, join: Option<String>, group: bool, ttl_min: u64, out: PathBuf) -> RoomOptions {
        let relays = if self.no_nostr || self.offline {
            vec![]
        } else if self.relays.is_empty() {
            signal::DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
        } else {
            self.relays.clone()
        };
        let trackers = if self.no_torrent || self.offline {
            vec![]
        } else if self.trackers.is_empty() {
            signal::DEFAULT_TRACKERS.iter().map(|s| s.to_string()).collect()
        } else {
            self.trackers.clone()
        };
        let mut ice = Vec::new();
        if !self.no_stun {
            let stun = if self.stun.is_empty() {
                DEFAULT_STUN.iter().map(|s| s.to_string()).collect()
            } else {
                self.stun.clone()
            };
            ice.push(RTCIceServer {
                urls: stun,
                ..Default::default()
            });
        }
        if let Some(turn) = &self.turn {
            ice.push(RTCIceServer {
                urls: vec![turn.clone()],
                username: self.turn_user.clone(),
                credential: self.turn_credential.clone(),
            });
        }
        RoomOptions {
            join,
            group,
            ttl_min,
            passphrase: self.passphrase.clone().filter(|p| !p.is_empty()),
            name: room::clean_name(&self.name),
            base: self.base.clone(),
            signal: signal::SignalConfig { relays, trackers },
            ice_servers: ice,
            relay_only: self.relay_only,
            out_dir: out,
            ttl: self.ttl.min(30 * 86_400),
        }
    }
}

fn print_qr(text: &str) {
    if let Ok(code) = qrcode::QrCode::new(text.as_bytes()) {
        let s = code
            .render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .quiet_zone(true)
            .build();
        eprintln!("{s}");
    }
}

fn human(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1 << 20 => format!("{:.1} KB", n as f64 / 1024.0),
        n if n < 1 << 30 => format!("{:.1} MB", n as f64 / 1048576.0),
        n => format!("{:.2} GB", n as f64 / 1073741824.0),
    }
}

fn who(name: &str, peer: &str) -> String {
    if name.is_empty() {
        format!("peer {peer}")
    } else {
        name.to_string()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Chat,
    Send,
    Receive,
}

/// Prints room events; returns when the room ends.
async fn drive(
    room: Room,
    mode: Mode,
    offline: bool,
    files: Vec<PathBuf>,
    mut events: mpsc::UnboundedReceiver<Event>,
    cmds: mpsc::UnboundedSender<RoomCmd>,
    cmd_rx: mpsc::UnboundedReceiver<RoomCmd>,
) -> Result<EndReason> {
    let mut room = room;
    room.set_manual(offline);
    let handle = tokio::spawn(room.run(cmd_rx));
    let mut pending_files = files.len();
    let mut offered = false;
    let mut downloads = 0usize;
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let interactive = mode == Mode::Chat || offline;
    loop {
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    Event::Link(url) => {
                        eprintln!("\nSend this link (it expires, and a one-to-one link works only once):\n");
                        println!("{url}");
                        print_qr(&url);
                    }
                    Event::Waiting => eprintln!("waiting for the other side… (Ctrl-C to cancel)"),
                    Event::Code(code) => {
                        eprintln!("\nShow this code to the other side (or paste theirs with /code <code>):\n");
                        println!("{code}");
                        print_qr(&code);
                    }
                    Event::Connected { peer, name, sas, sas_names } => {
                        eprintln!("\n✓ encrypted session with {} (X25519 + ML-KEM-768)", who(&name, &peer));
                        if !sas.is_empty() {
                            eprintln!("  compare these emojis with them: {sas}  ({sas_names})");
                            if mode == Mode::Chat { eprintln!("  type /ok if they match, /no if not"); }
                        }
                        if mode == Mode::Send && !offered {
                            offered = true;
                            for f in &files { let _ = cmds.send(RoomCmd::SendFile(f.clone())); }
                        }
                    }
                    Event::Path { peer, direct } => eprintln!(
                        "  {} path to {peer}", if direct { "direct" } else { "no direct path, relayed (still end-to-end encrypted)" }),
                    Event::Verified { peer } => eprintln!("  ✓ verified by both sides ({peer})"),
                    Event::Message { peer, name, text } => println!("{}: {text}", who(&name, &peer)),
                    Event::FileOffer { peer, name, id, file, size } => {
                        eprintln!("{} offers {file} ({}) — /get {id}", who(&name, &peer), human(size));
                        if mode == Mode::Receive { downloads += 1; let _ = cmds.send(RoomCmd::Get(id)); }
                    }
                    Event::FileProgress { done, total } => {
                        eprint!("\r  receiving… {:>3}% ({} / {})", done * 100 / total.max(1), human(done), human(total));
                        let _ = std::io::stderr().flush();
                    }
                    Event::FileSaved { path } => {
                        eprintln!("\n  ✓ saved {} (SHA-256 verified)", path.display());
                        downloads = downloads.saturating_sub(1);
                    }
                    Event::FileFailed { id, reason } => {
                        eprintln!("\n  ✗ {id}: {reason}");
                        downloads = downloads.saturating_sub(1);
                    }
                    Event::FileServed { id, peer } => {
                        eprintln!("  ✓ {id} delivered to {peer}");
                        pending_files = pending_files.saturating_sub(1);
                        if mode == Mode::Send && pending_files == 0 {
                            // Let the last chunks drain before saying goodbye.
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            let _ = cmds.send(RoomCmd::End);
                        }
                    }
                    Event::Left { peer, name } => eprintln!("{} left", who(&name, &peer)),
                    Event::Notice(n) => eprintln!("  {n}"),
                    Event::Ended => {}
                }
                let _ = downloads;
            }
            line = stdin.next_line(), if interactive => {
                let Ok(Some(line)) = line else {
                    if mode == Mode::Chat { let _ = cmds.send(RoomCmd::End); }
                    continue;
                };
                let l = line.trim();
                let cmd = if let Some(rest) = l.strip_prefix("/file ") {
                    RoomCmd::SendFile(PathBuf::from(rest.trim()))
                } else if let Some(rest) = l.strip_prefix("/get ") {
                    RoomCmd::Get(rest.trim().to_string())
                } else if let Some(rest) = l.strip_prefix("/code ") {
                    RoomCmd::Manual(rest.trim().to_string())
                } else if l.starts_with("hushcode:") {
                    RoomCmd::Manual(l.to_string())
                } else if l == "/ok" {
                    RoomCmd::Verify(true)
                } else if l == "/no" {
                    RoomCmd::Verify(false)
                } else if l == "/quit" || l == "/end" {
                    RoomCmd::End
                } else if l == "/help" {
                    eprintln!("/file PATH  send a file · /get ID  download an offered file · /ok /no  emojis match or not · /code CODE  paste a code · /quit  end and erase");
                    continue;
                } else {
                    RoomCmd::Text(line)
                };
                let _ = cmds.send(cmd);
            }
            _ = tokio::signal::ctrl_c() => { let _ = cmds.send(RoomCmd::End); }
        }
    }
    handle.await.context("room task")?
}

fn read_secret(prompt: &str) -> Result<String> {
    eprint!("{prompt}: ");
    std::io::stderr().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim_end_matches(['\r', '\n']).to_string())
}

fn backup(action: BackupCmd) -> Result<()> {
    match action {
        BackupCmd::Inspect { file, passphrase } => {
            let data = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            if !hush_core::backup::is_backup(&data) {
                bail!("{} is not a hush backup", file.display());
            }
            let pass = match passphrase {
                Some(p) => p,
                None => read_secret("backup passphrase")?,
            };
            let plain = hush_core::backup::import(&pass, &data)
                .map_err(|_| anyhow::anyhow!("wrong passphrase or modified file"))?;
            let snap: Value = serde_json::from_slice(&plain).context("backup content")?;
            if let Some(seed) = snap["identity"]["seed"].as_str().and_then(util::from_b64u) {
                let seed: [u8; 32] = seed.try_into().map_err(|_| anyhow::anyhow!("bad identity seed"))?;
                let id = hush_core::identity::Identity::from_seed(seed);
                println!("identifier: {}", id.public().id());
                if let Some(n) = snap["identity"]["name"].as_str().filter(|n| !n.is_empty()) {
                    println!("name:       {n}");
                }
            } else {
                println!("identifier: (none)");
            }
            let contacts = snap["contacts"].as_array().cloned().unwrap_or_default();
            println!("contacts:   {}", contacts.len());
            for c in contacts {
                let key = c["key"].as_str().unwrap_or("");
                let kept = snap["history"][key].as_array().map_or(0, Vec::len);
                let verified = c["bundle"]
                    .as_str()
                    .and_then(util::from_b64u)
                    .and_then(|b| hush_core::bundle::Bundle::parse_stored(&b).ok())
                    .is_some_and(|b| b.identity.id() == c["id"].as_str().unwrap_or(""));
                println!(
                    "  {} {}  {}{}",
                    if verified { "✓" } else { "✗" },
                    c["name"].as_str().unwrap_or("?"),
                    c["id"].as_str().unwrap_or("?"),
                    if kept > 0 {
                        format!("  ({kept} messages kept)")
                    } else {
                        String::new()
                    }
                );
            }
            Ok(())
        }
        BackupCmd::Rekey {
            file,
            out,
            passphrase,
            new_passphrase,
        } => {
            let data = std::fs::read(&file)?;
            let pass = match passphrase {
                Some(p) => p,
                None => read_secret("current backup passphrase")?,
            };
            let plain = hush_core::backup::import(&pass, &data)
                .map_err(|_| anyhow::anyhow!("wrong passphrase or modified file"))?;
            let new = match new_passphrase {
                Some(p) => p,
                None => read_secret("new backup passphrase")?,
            };
            if new.chars().count() < 10 {
                bail!("the new passphrase must be at least 10 characters");
            }
            let sealed =
                hush_core::backup::export(&new, hush_core::vault::KdfParams::DEFAULT, &plain, &mut util::rng())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            std::fs::write(&out, sealed)?;
            eprintln!("wrote {}", out.display());
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let (mode, opts, offline, files) = match cli.command {
        Command::New {
            group,
            expires,
            with_passphrase,
            base,
        } => {
            let mode = if group {
                hush_core::link::Mode::Group
            } else {
                hush_core::link::Mode::Direct
            };
            let link = hush_core::link::Link::generate(
                mode,
                util::now_ms(),
                expires.max(1),
                with_passphrase,
                &mut util::rng(),
            );
            println!("{}", link.url(&base));
            return Ok(());
        }
        Command::Backup { action } => return backup(action),
        Command::Chat {
            link,
            group,
            expires,
            out,
            net,
        } => {
            let offline = net.offline;
            (Mode::Chat, net.options(link, group, expires, out), offline, vec![])
        }
        Command::Send {
            files,
            link,
            expires,
            net,
        } => {
            for f in &files {
                if !f.is_file() {
                    bail!("{} is not a file", f.display());
                }
            }
            let offline = net.offline;
            (
                Mode::Send,
                net.options(link, false, expires, PathBuf::from(".")),
                offline,
                files,
            )
        }
        Command::Receive { link, out, net } => {
            std::fs::create_dir_all(&out)?;
            let offline = net.offline;
            (Mode::Receive, net.options(Some(link), false, 60, out), offline, vec![])
        }
    };
    let mut opts = opts;
    if let Some(link) = &opts.join {
        let parsed = hush_core::link::Link::parse(link).map_err(|_| anyhow::anyhow!("not a hush link"))?;
        if parsed.passphrase && opts.passphrase.is_none() {
            opts.passphrase = Some(read_secret("this link needs a passphrase")?);
        }
    }
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let room = Room::new(opts, ev_tx)?;
    let reason = drive(room, mode, offline, files, ev_rx, cmd_tx, cmd_rx).await?;
    match reason {
        EndReason::Ended => eprintln!("ended: keys erased, nothing was saved"),
        EndReason::PeerLeft => eprintln!("the other side ended the conversation: keys erased"),
        EndReason::Taken => {
            eprintln!("this link was already used by someone else.");
            eprintln!("if it was not the person you expected, the link was intercepted: tell them another way.");
            std::process::exit(3);
        }
        EndReason::Expired => {
            eprintln!("this link has expired");
            std::process::exit(4);
        }
    }
    // Exit now: a pending read on stdin would otherwise keep the runtime alive.
    std::process::exit(0);
}
