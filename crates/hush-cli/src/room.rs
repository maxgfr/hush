//! A conversation from a link, as in `web/src/app/{conversation,chat,peer}.ts`: the link
//! handshake over signaling, one ratcheting session per peer (one-to-one, or a full mesh for a
//! group), carried over WebRTC or relayed through the signaling transports when no direct path
//! exists, and the application messages (`PROTOCOL.md`).

use crate::rtc::{self, RtcCmd, RtcEvent, RtcOptions};
use crate::signal::{Channel, ChannelId, Rendezvous, SignalConfig};
use crate::util::{new_id, now_ms, rng};
use anyhow::{Result, anyhow, bail};
use hush_core::envelope::{PeerTag, kind};
use hush_core::handshake::{LinkCreator, LinkCreatorPending, LinkJoiner};
use hush_core::link::{Link, LinkKeys, Mode};
use hush_core::session::{Session, lane};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use webrtc::peer_connection::RTCIceServer;

pub const CHUNK_SIZE: usize = 48 * 1024;
const RESEND: Duration = Duration::from_secs(6);
const DIRECT_TIMEOUT: Duration = Duration::from_secs(20);
const IN_FLIGHT: usize = 64;
const MAX_GROUP: usize = 16;
/// Handshakes waiting for their last message at once, and for how long.
const MAX_PENDING: usize = 4;
const PENDING_TIMEOUT: Duration = Duration::from_secs(45);

pub struct RoomOptions {
    /// Join this link, or create one.
    pub join: Option<String>,
    pub group: bool,
    pub ttl_min: u64,
    pub passphrase: Option<String>,
    pub name: String,
    pub base: String,
    pub signal: SignalConfig,
    pub ice_servers: Vec<RTCIceServer>,
    pub relay_only: bool,
    pub out_dir: PathBuf,
    /// Message timer for what we send, in seconds (0 = off).
    pub ttl: u64,
}

/// What the room tells the terminal.
#[derive(Debug)]
pub enum Event {
    Link(String),
    Waiting,
    /// A hand-carried code to show (offline mode).
    Code(String),
    Connected {
        peer: String,
        name: String,
        sas: String,
        sas_names: String,
    },
    Path {
        peer: String,
        direct: bool,
    },
    Verified {
        peer: String,
    },
    Message {
        peer: String,
        name: String,
        text: String,
    },
    FileOffer {
        peer: String,
        name: String,
        id: String,
        file: String,
        size: u64,
    },
    FileProgress {
        done: u64,
        total: u64,
    },
    FileSaved {
        path: PathBuf,
    },
    FileFailed {
        id: String,
        reason: String,
    },
    FileServed {
        id: String,
        peer: String,
    },
    Left {
        peer: String,
        name: String,
    },
    Notice(String),
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Ended,
    PeerLeft,
    Taken,
    Expired,
}

/// What the terminal asks of the room.
pub enum Command {
    Text(String),
    SendFile(PathBuf),
    Get(String),
    Verify(bool),
    Manual(String),
    End,
}

struct OutFile {
    path: PathBuf,
    size: u64,
}

struct Upload {
    id: String,
    file: std::fs::File,
    index: u32,
    sent: u64,
    size: u64,
}

struct Download {
    id: String,
    name: String,
    size: u64,
    sha256: String,
    got: u64,
    next: u32,
    hasher: Sha256,
    file: Option<(std::fs::File, PathBuf)>,
}

struct Peer {
    session: Session,
    name: String,
    rtc: mpsc::UnboundedSender<RtcCmd>,
    direct: bool,
    relayed: bool,
    signal: ChannelId,
    since: Instant,
    verified_me: bool,
    verified_peer: bool,
    uploads: Vec<Upload>,
    in_flight: usize,
    offers: HashMap<String, (String, u64, String)>,
    downloads: HashMap<String, Download>,
    /// Downloads asked for before the direct link opened.
    wanted: Vec<String>,
}

type RtcTx = mpsc::UnboundedSender<(PeerTag, RtcEvent)>;
type RtcRx = mpsc::UnboundedReceiver<(PeerTag, RtcEvent)>;

pub fn tag_hex(t: &PeerTag) -> String {
    hex::encode(&t[..4])
}

fn hello(name: &str, sdp: Option<&str>) -> Vec<u8> {
    let mut v = json!({"v": 1, "name": name});
    if let Some(s) = sdp {
        v["sdp"] = json!(s);
    }
    v.to_string().into_bytes()
}

fn parse_hello(b: &[u8]) -> (String, Option<String>) {
    let v: Value = serde_json::from_slice(b).unwrap_or(Value::Null);
    let name = v["name"].as_str().map(clean_name).unwrap_or_default();
    (name, v["sdp"].as_str().map(str::to_string))
}

/// Drops control, zero-width and bidi-override characters from a peer-chosen string.
pub fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| {
            let u = *c as u32;
            u >= 0x20
                && !(0x7f..=0x9f).contains(&u)
                && !matches!(u, 0x200b..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069 | 0xfeff)
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(64)
        .collect()
}

/// How a peer appears in notices: its name, or its tag when it has none.
fn who_label(name: &str, tag_hex: &str) -> String {
    if name.is_empty() {
        tag_hex.to_string()
    } else {
        name.to_string()
    }
}

fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .filter(|c| !matches!(*c as u32, 0x202a..=0x202e | 0x2066..=0x2069))
        .collect()
}

pub struct Room {
    opts: RoomOptions,
    rdv: Rendezvous,
    keys: LinkKeys,
    link_ch: ChannelId,
    joiner: Option<LinkJoiner>,
    hs1: Option<Vec<u8>>,
    last_hs1: Instant,
    awaiting: HashMap<PeerTag, (LinkCreator, String, mpsc::UnboundedSender<RtcCmd>, Instant)>,
    answered: HashSet<PeerTag>,
    peers: HashMap<PeerTag, Peer>,
    sessions_by_channel: HashMap<ChannelId, PeerTag>,
    rtc_events: (RtcTx, RtcRx),
    events: mpsc::UnboundedSender<Event>,
    outgoing: HashMap<String, OutFile>,
    expires_ms: u64,
    creator: bool,
    ended: bool,
    end_reason: Option<EndReason>,
    locked: bool,
    pending_manual: Vec<(ChannelId, hush_core::envelope::Envelope)>,
}

impl Room {
    pub fn new(opts: RoomOptions, events: mpsc::UnboundedSender<Event>) -> Result<Self> {
        let pass = opts.passphrase.clone().filter(|p| !p.is_empty());
        let (link, creator) = match &opts.join {
            Some(input) => (Link::parse(input).map_err(crate::util::link_error)?, false),
            None => (
                Link::generate(
                    if opts.group { Mode::Group } else { Mode::Direct },
                    now_ms(),
                    opts.ttl_min,
                    pass.is_some(),
                    &mut rng(),
                ),
                true,
            ),
        };
        if link.is_expired(now_ms()) {
            bail!("this link has expired");
        }
        if link.passphrase && pass.is_none() {
            bail!("this link needs a passphrase (--passphrase or HUSH_PASSPHRASE)");
        }
        let keys = link
            .keys(if link.passphrase { pass.as_deref() } else { None })
            .map_err(|_| anyhow!("bad passphrase"))?;
        let mut rdv = Rendezvous::new(&opts.signal);
        let topic_keys = keys.clone();
        let link_ch = rdv.add(Channel::new(*keys.env_key, move |d| topic_keys.topic(d)));
        let expires_ms = link.expires_min * 60_000;
        if creator {
            let _ = events.send(Event::Link(link.url(&opts.base)));
        }
        let mut room = Self {
            rdv,
            link_ch,
            joiner: None,
            hs1: None,
            last_hs1: Instant::now(),
            awaiting: HashMap::new(),
            answered: HashSet::new(),
            peers: HashMap::new(),
            sessions_by_channel: HashMap::new(),
            rtc_events: mpsc::unbounded_channel(),
            events,
            outgoing: HashMap::new(),
            expires_ms,
            creator,
            ended: false,
            end_reason: None,
            locked: false,
            pending_manual: Vec::new(),
            opts,
            keys,
        };
        room.opts.group = link.mode == Mode::Group;
        if !creator {
            let (state, hs1) = LinkJoiner::start(&room.keys, &hello(&room.opts.name, None), &mut rng());
            room.joiner = Some(state);
            room.send_hs1(&hs1);
            room.hs1 = Some(hs1);
        }
        let _ = room.events.send(Event::Waiting);
        Ok(room)
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    fn send_hs1(&mut self, hs1: &[u8]) {
        self.last_hs1 = Instant::now();
        if let Some(code) = self.rdv.send(self.link_ch, kind::HS1, hs1, None)
            && self.rdv.manual
        {
            self.emit(Event::Code(code));
        }
    }

    fn send_link(&self, kind: u8, body: &[u8], to: PeerTag) {
        if let Some(code) = self.rdv.send(self.link_ch, kind, body, Some(to))
            && self.rdv.manual
            && matches!(kind, kind::HS2 | kind::HS3)
        {
            self.emit(Event::Code(code));
        }
    }

    pub fn set_manual(&mut self, on: bool) {
        self.rdv.manual = on;
        if on && let Some(hs1) = self.hs1.clone() {
            self.send_hs1(&hs1);
        }
    }

    fn rtc_opts(&self) -> RtcOptions {
        RtcOptions {
            ice_servers: self.opts.ice_servers.clone(),
            relay_only: self.opts.relay_only,
        }
    }

    fn live(&self) -> usize {
        self.peers.len()
    }

    /// Runs until the conversation ends.
    pub async fn run(mut self, mut cmds: mpsc::UnboundedReceiver<Command>) -> Result<EndReason> {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            if self.ended {
                break;
            }
            tokio::select! {
                env = self.rdv.next() => {
                    let Some((ch, env)) = env else { break };
                    self.on_envelope(ch, env).await;
                }
                ev = self.rtc_events.1.recv() => {
                    if let Some((tag, ev)) = ev { self.on_rtc(tag, ev); }
                }
                cmd = cmds.recv() => {
                    match cmd {
                        None | Some(Command::End) => { self.end(EndReason::Ended); }
                        Some(c) => self.on_command(c),
                    }
                    for (ch, env) in std::mem::take(&mut self.pending_manual) {
                        self.on_envelope(ch, env).await;
                    }
                }
                _ = tick.tick() => self.on_tick(),
            }
        }
        // Give farewells a moment to leave.
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(self.reason())
    }

    fn reason(&self) -> EndReason {
        self.end_reason.unwrap_or(EndReason::Ended)
    }

    fn on_tick(&mut self) {
        self.rdv.rotate();
        self.awaiting.retain(|_, (_, _, rtc, since)| {
            let alive = since.elapsed() < PENDING_TIMEOUT;
            if !alive {
                let _ = rtc.send(RtcCmd::Close);
            }
            alive
        });
        let now = now_ms();
        let waiting = self.peers.is_empty();
        if waiting && now >= self.expires_ms && self.awaiting.is_empty() {
            self.end(EndReason::Expired);
            return;
        }
        if waiting && self.hs1.is_some() && self.last_hs1.elapsed() >= RESEND && !self.rdv.manual {
            let hs1 = self.hs1.clone().expect("checked");
            self.send_hs1(&hs1);
        }
        let tags: Vec<PeerTag> = self.peers.keys().copied().collect();
        for tag in tags {
            let mut out = Vec::new();
            if let Some(p) = self.peers.get_mut(&tag) {
                out = p.session.poll(now, &mut rng());
                if !p.direct && !p.relayed && p.since.elapsed() >= DIRECT_TIMEOUT {
                    p.relayed = true;
                    let _ = self.events.send(Event::Path {
                        peer: tag_hex(&tag),
                        direct: false,
                    });
                }
            }
            for (l, f) in out {
                self.send_frame(&tag, l, f);
            }
            self.pump(&tag);
        }
    }

    // ── handshake ───────────────────────────────────────────────────────────

    async fn on_envelope(&mut self, ch: ChannelId, env: hush_core::envelope::Envelope) {
        if ch != self.link_ch {
            if env.kind == kind::SESSION
                && let Some(tag) = self.sessions_by_channel.get(&ch).copied()
            {
                self.on_frame(&tag, &env.body);
            }
            return;
        }
        let to_us = env.to != hush_core::envelope::BROADCAST;
        match env.kind {
            kind::HS1 => self.on_hs1(env.from, &env.body).await,
            kind::HS2 if to_us => self.on_hs2(env.from, &env.body).await,
            kind::HS3 if to_us => self.on_hs3(env.from, &env.body).await,
            kind::TAKEN if to_us && !self.opts.group && self.peers.is_empty() => self.end(EndReason::Taken),
            _ => {}
        }
    }

    fn is_member(&self) -> bool {
        self.creator || !self.peers.is_empty()
    }

    async fn on_hs1(&mut self, from: PeerTag, hs1: &[u8]) {
        if !self.is_member() || self.answered.contains(&from) || self.peers.contains_key(&from) {
            return;
        }
        let full = if self.opts.group {
            // Someone holding the link could otherwise open connections without end.
            self.locked || self.live() >= MAX_GROUP - 1 || self.awaiting.len() >= MAX_PENDING
        } else {
            !self.peers.is_empty() || !self.awaiting.is_empty()
        };
        if full {
            if !self.opts.group {
                self.send_link(kind::TAKEN, &[], from);
            }
            return;
        }
        let Ok((pending, payload)) = LinkCreatorPending::read_hs1(&self.keys, hs1) else {
            return;
        };
        self.answered.insert(from);
        let (name, _) = parse_hello(&payload);
        let rtc = rtc::spawn(from, self.rtc_opts(), self.rtc_events.0.clone());
        let (tx, rx) = oneshot::channel();
        let _ = rtc.send(RtcCmd::CreateOffer(tx));
        let Ok(Ok(offer)) = rx.await else {
            let _ = rtc.send(RtcCmd::Close);
            return;
        };
        let Ok((creator, hs2)) = pending.write_hs2(&hello(&self.opts.name, Some(&offer)), &mut rng()) else {
            let _ = rtc.send(RtcCmd::Close);
            return;
        };
        self.awaiting.insert(from, (creator, name, rtc, Instant::now()));
        self.send_link(kind::HS2, &hs2, from);
    }

    async fn on_hs3(&mut self, from: PeerTag, hs3: &[u8]) {
        let Some((creator, name, rtc, _)) = self.awaiting.remove(&from) else {
            return;
        };
        let Ok((payload, est)) = creator.read_hs3(hs3) else {
            let _ = rtc.send(RtcCmd::Close);
            return;
        };
        let (_, sdp) = parse_hello(&payload);
        if let Some(sdp) = sdp {
            let _ = rtc.send(RtcCmd::AcceptAnswer(sdp));
        }
        self.add_peer(from, &est, name, rtc);
    }

    async fn on_hs2(&mut self, from: PeerTag, hs2: &[u8]) {
        if self.answered.contains(&from) || (!self.opts.group && !self.peers.is_empty()) || self.live() >= MAX_GROUP - 1
        {
            return;
        }
        let Some(joiner) = &self.joiner else { return };
        let Ok((ready, payload)) = joiner.read_hs2(hs2) else {
            return;
        };
        self.answered.insert(from);
        let (name, sdp) = parse_hello(&payload);
        let Some(sdp) = sdp else { return };
        let rtc = rtc::spawn(from, self.rtc_opts(), self.rtc_events.0.clone());
        let (tx, rx) = oneshot::channel();
        let _ = rtc.send(RtcCmd::AcceptOffer(sdp, tx));
        let Ok(Ok(answer)) = rx.await else {
            let _ = rtc.send(RtcCmd::Close);
            return;
        };
        let (hs3, est) = ready.write_hs3(json!({"v": 1, "sdp": answer}).to_string().as_bytes());
        self.send_link(kind::HS3, &hs3, from);
        self.add_peer(from, &est, name, rtc);
    }

    fn add_peer(
        &mut self,
        tag: PeerTag,
        est: &hush_core::handshake::Established,
        name: String,
        rtc: mpsc::UnboundedSender<RtcCmd>,
    ) {
        let session = Session::new(est, now_ms());
        let sk = session.signal_keys();
        let signal = self.rdv.add(Channel::new(sk.env_key(), move |d| sk.topic(d)));
        self.sessions_by_channel.insert(signal, tag);
        let sas = est.sas.map(|s| hush_core::sas::render(&s)).unwrap_or_default();
        let sas_names = est.sas.map(|s| hush_core::sas::names(&s)).unwrap_or_default();
        self.peers.insert(
            tag,
            Peer {
                session,
                name: name.clone(),
                rtc,
                direct: false,
                relayed: false,
                signal,
                since: Instant::now(),
                verified_me: false,
                verified_peer: false,
                uploads: Vec::new(),
                in_flight: 0,
                offers: HashMap::new(),
                downloads: HashMap::new(),
                wanted: Vec::new(),
            },
        );
        self.emit(Event::Connected {
            peer: tag_hex(&tag),
            name,
            sas,
            sas_names,
        });
        self.send_app(&tag, lane::CTRL, json!({"t": "name", "name": self.opts.name}));
    }

    // ── frames ──────────────────────────────────────────────────────────────

    fn on_rtc(&mut self, tag: PeerTag, ev: RtcEvent) {
        match ev {
            RtcEvent::Open => {
                let mut wanted = Vec::new();
                if let Some(p) = self.peers.get_mut(&tag) {
                    p.direct = true;
                    wanted = std::mem::take(&mut p.wanted);
                    self.emit(Event::Path {
                        peer: tag_hex(&tag),
                        direct: true,
                    });
                }
                for id in wanted {
                    self.request_file(&id);
                }
                self.pump(&tag);
            }
            RtcEvent::Frame(frame) => self.on_frame(&tag, &frame),
            RtcEvent::Closed => {
                if let Some(p) = self.peers.get_mut(&tag) {
                    p.direct = false;
                }
            }
            RtcEvent::Sent => {
                if let Some(p) = self.peers.get_mut(&tag) {
                    p.in_flight = p.in_flight.saturating_sub(1);
                }
                self.pump(&tag);
            }
        }
    }

    fn send_frame(&self, tag: &PeerTag, l: u8, frame: Vec<u8>) {
        let Some(p) = self.peers.get(tag) else { return };
        if p.direct {
            let _ = p.rtc.send(RtcCmd::Send(usize::from(l), frame));
        } else if l != lane::FILE {
            self.rdv.send(p.signal, kind::SESSION, &frame, None);
        }
    }

    fn send_app(&mut self, tag: &PeerTag, l: u8, v: Value) {
        let Some(p) = self.peers.get_mut(tag) else { return };
        let Ok(frame) = p.session.seal(l, v.to_string().as_bytes()) else {
            return;
        };
        self.send_frame(tag, l, frame);
    }

    fn broadcast_app(&mut self, l: u8, v: Value) {
        let tags: Vec<PeerTag> = self.peers.keys().copied().collect();
        for t in tags {
            self.send_app(&t, l, v.clone());
        }
    }

    fn on_frame(&mut self, tag: &PeerTag, frame: &[u8]) {
        let Some(p) = self.peers.get_mut(tag) else { return };
        let Ok(opened) = p.session.open(frame, now_ms(), &mut rng()) else {
            return;
        };
        for (l, f) in opened.replies {
            self.send_frame(tag, l, f);
        }
        for (l, data) in opened.messages {
            self.on_message(tag, l, &data);
        }
        if opened.closed {
            self.drop_peer(tag, true);
        }
    }

    fn drop_peer(&mut self, tag: &PeerTag, by_peer: bool) {
        let Some(p) = self.peers.remove(tag) else { return };
        let _ = p.rtc.send(RtcCmd::Close);
        self.rdv.remove(p.signal);
        self.sessions_by_channel.remove(&p.signal);
        self.emit(Event::Left {
            peer: tag_hex(tag),
            name: p.name,
        });
        if !self.opts.group {
            self.end(if by_peer { EndReason::PeerLeft } else { EndReason::Ended });
        }
    }

    fn on_message(&mut self, tag: &PeerTag, l: u8, data: &[u8]) {
        if l == lane::FILE {
            self.on_chunk(tag, data);
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(data) else {
            return;
        };
        let peer = tag_hex(tag);
        let name = self.peers.get(tag).map(|p| p.name.clone()).unwrap_or_default();
        match (l, v["t"].as_str()) {
            (lane::CTRL, Some("name")) => {
                if let (Some(p), Some(n)) = (self.peers.get_mut(tag), v["name"].as_str()) {
                    let n = clean_name(n);
                    if n != p.name {
                        let notice = format!("{} is now called {n}", who_label(&p.name, &peer));
                        p.name = n;
                        self.emit(Event::Notice(notice));
                    }
                }
            }
            (lane::CTRL, Some("verified")) => {
                if let Some(p) = self.peers.get_mut(tag) {
                    p.verified_peer = true;
                    if p.verified_me {
                        self.emit(Event::Verified { peer });
                    }
                }
            }
            (lane::CTRL, Some("lock")) => {
                if let Some(locked) = v["locked"].as_bool()
                    && locked != self.locked
                {
                    self.locked = locked;
                    let what = if locked { "locked the room" } else { "unlocked the room" };
                    self.emit(Event::Notice(format!("{} {what}", who_label(&name, &peer))));
                }
            }
            (lane::CHAT, Some("msg")) => {
                if let Some(text) = v["text"].as_str() {
                    self.emit(Event::Message {
                        peer,
                        name,
                        text: clean_text(text),
                    });
                }
            }
            (lane::CHAT, Some("file")) => {
                let (Some(id), Some(file), Some(size), Some(sha)) = (
                    v["id"].as_str(),
                    v["name"].as_str(),
                    v["size"].as_u64(),
                    v["sha256"].as_str(),
                ) else {
                    return;
                };
                if id.len() != 16 || sha.len() != 64 {
                    return;
                }
                let file = sanitize_file_name(file);
                if let Some(p) = self.peers.get_mut(tag) {
                    p.offers.insert(id.to_string(), (file.clone(), size, sha.to_string()));
                }
                self.emit(Event::FileOffer {
                    peer,
                    name,
                    id: id.to_string(),
                    file,
                    size,
                });
            }
            (lane::CHAT, Some("file-get")) => {
                if let Some(id) = v["id"].as_str() {
                    self.start_upload(tag, id);
                }
            }
            (lane::CHAT, Some("file-end")) => {
                if let Some(id) = v["id"].as_str().filter(|id| self.outgoing.contains_key(*id)) {
                    self.emit(Event::FileServed {
                        id: id.to_string(),
                        peer,
                    });
                }
            }
            _ => {}
        }
    }

    // ── files ───────────────────────────────────────────────────────────────

    fn offer_file(&mut self, path: PathBuf) -> Result<()> {
        let mut f = std::fs::File::open(&path)?;
        let size = f.metadata()?.len();
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        let id = new_id();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
        let mime = guess_mime(&name);
        let mut offer = json!({"t": "file", "id": id, "ts": now_ms(), "name": name, "size": size, "mime": mime, "sha256": hex::encode(h.finalize())});
        if self.opts.ttl > 0 {
            offer["ttl"] = json!(self.opts.ttl);
        }
        self.outgoing.insert(id.clone(), OutFile { path, size });
        self.broadcast_app(lane::CHAT, offer);
        self.emit(Event::Notice(format!("offered {name} ({size} bytes) as {id}")));
        Ok(())
    }

    fn start_upload(&mut self, tag: &PeerTag, id: &str) {
        let Some(out) = self.outgoing.get(id) else { return };
        let Ok(file) = std::fs::File::open(&out.path) else {
            return;
        };
        let size = out.size;
        // Queued even if our side has not seen the link open yet: `pump` starts once it has.
        if let Some(p) = self.peers.get_mut(tag) {
            p.uploads.push(Upload {
                id: id.to_string(),
                file,
                index: 0,
                sent: 0,
                size,
            });
        }
        self.pump(tag);
    }

    /// Sends file chunks while the link's send window has room.
    fn pump(&mut self, tag: &PeerTag) {
        let mut done = Vec::new();
        let mut frames = Vec::new();
        {
            let Some(p) = self.peers.get_mut(tag) else { return };
            if !p.direct {
                return;
            }
            while p.in_flight < IN_FLIGHT {
                let Some(up) = p.uploads.first_mut() else { break };
                if up.sent >= up.size {
                    done.push(p.uploads.remove(0).id);
                    continue;
                }
                let mut buf = vec![0u8; CHUNK_SIZE.min((up.size - up.sent) as usize)];
                if up.file.read_exact(&mut buf).is_err() {
                    p.uploads.remove(0);
                    continue;
                }
                let mut chunk = Vec::with_capacity(21 + buf.len());
                chunk.push(1u8);
                chunk.extend_from_slice(up.id.as_bytes());
                chunk.extend_from_slice(&up.index.to_be_bytes());
                chunk.extend_from_slice(&buf);
                up.index += 1;
                up.sent += buf.len() as u64;
                let Ok(frame) = p.session.seal(lane::FILE, &chunk) else {
                    break;
                };
                p.in_flight += 1;
                frames.push(frame);
            }
            if let Some(up) = p.uploads.first()
                && up.sent >= up.size
                && !done.contains(&up.id)
            {
                done.push(p.uploads.remove(0).id);
            }
        }
        for f in frames {
            if let Some(p) = self.peers.get(tag) {
                let _ = p.rtc.send(RtcCmd::SendTracked(usize::from(lane::FILE), f));
            }
        }
        // Finished uploads are reported when the receiver confirms (`file-end`), not here.
        let _ = done;
    }

    fn request_file(&mut self, id: &str) {
        let tags: Vec<PeerTag> = self.peers.keys().copied().collect();
        for tag in tags {
            let Some(p) = self.peers.get_mut(&tag) else { continue };
            if !p.offers.contains_key(id) {
                continue;
            }
            if !p.direct {
                // The offer may come through the relays before the direct link opens: wait for it.
                if p.relayed {
                    self.emit(Event::FileFailed {
                        id: id.to_string(),
                        reason: "files need a direct connection".into(),
                    });
                } else if !p.wanted.iter().any(|w| w == id) {
                    p.wanted.push(id.to_string());
                }
                return;
            }
            let Some((name, size, sha)) = p.offers.remove(id) else {
                continue;
            };
            let dl = Download {
                id: id.to_string(),
                name,
                size,
                sha256: sha,
                got: 0,
                next: 0,
                hasher: Sha256::new(),
                file: None,
            };
            p.downloads.insert(id.to_string(), dl);
            if size == 0 {
                self.finish_download(&tag, id);
            } else {
                self.send_app(&tag, lane::CHAT, json!({"t": "file-get", "id": id}));
            }
            return;
        }
        self.emit(Event::FileFailed {
            id: id.to_string(),
            reason: "no such file".into(),
        });
    }

    fn on_chunk(&mut self, tag: &PeerTag, data: &[u8]) {
        if data.len() < 21 || data[0] != 1 {
            return;
        }
        let Ok(id) = std::str::from_utf8(&data[1..17]) else {
            return;
        };
        let id = id.to_string();
        let index = u32::from_be_bytes([data[17], data[18], data[19], data[20]]);
        let body = &data[21..];
        let out_dir = self.opts.out_dir.clone();
        let Some(p) = self.peers.get_mut(tag) else { return };
        let Some(dl) = p.downloads.get_mut(&id) else { return };
        if index != dl.next || dl.got + body.len() as u64 > dl.size {
            let reason = "chunks out of order".to_string();
            if let Some(dl) = p.downloads.remove(&id)
                && let Some((_, path)) = dl.file
            {
                let _ = std::fs::remove_file(path);
            }
            self.emit(Event::FileFailed { id, reason });
            return;
        }
        if dl.file.is_none() {
            match create_unique(&out_dir, &dl.name) {
                Ok(x) => dl.file = Some(x),
                Err(e) => {
                    let reason = e.to_string();
                    p.downloads.remove(&id);
                    self.emit(Event::FileFailed { id, reason });
                    return;
                }
            }
        }
        if let Some((f, _)) = dl.file.as_mut() {
            let _ = f.write_all(body);
        }
        dl.hasher.update(body);
        dl.got += body.len() as u64;
        dl.next += 1;
        let (got, total) = (dl.got, dl.size);
        self.emit(Event::FileProgress { done: got, total });
        if got == total {
            self.finish_download(tag, &id);
        }
    }

    fn finish_download(&mut self, tag: &PeerTag, id: &str) {
        let out_dir = self.opts.out_dir.clone();
        let Some(p) = self.peers.get_mut(tag) else { return };
        let Some(dl) = p.downloads.remove(id) else { return };
        let (file, path) = match dl.file {
            Some(x) => x,
            None => match create_unique(&out_dir, &dl.name) {
                Ok(x) => x,
                Err(e) => {
                    self.emit(Event::FileFailed {
                        id: id.to_string(),
                        reason: e.to_string(),
                    });
                    return;
                }
            },
        };
        drop(file);
        if hex::encode(dl.hasher.finalize()) == dl.sha256 {
            self.emit(Event::FileSaved { path });
            self.send_app(tag, lane::CHAT, json!({"t": "file-end", "id": dl.id}));
        } else {
            let _ = std::fs::remove_file(&path);
            self.emit(Event::FileFailed {
                id: dl.id,
                reason: "SHA-256 mismatch: file discarded".into(),
            });
        }
    }

    // ── commands ────────────────────────────────────────────────────────────

    fn on_command(&mut self, c: Command) {
        match c {
            Command::Text(text) => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return;
                }
                let mut v = json!({"t": "msg", "id": new_id(), "ts": now_ms(), "text": text});
                if self.opts.ttl > 0 {
                    v["ttl"] = json!(self.opts.ttl);
                }
                self.broadcast_app(lane::CHAT, v);
            }
            Command::SendFile(path) => {
                if let Err(e) = self.offer_file(path) {
                    self.emit(Event::Notice(format!("cannot send: {e}")));
                }
            }
            Command::Get(id) => self.request_file(&id),
            Command::Verify(ok) => {
                let tags: Vec<PeerTag> = self.peers.keys().copied().collect();
                for tag in tags {
                    if !ok {
                        self.emit(Event::Notice("the emojis did not match: connection closed".into()));
                        self.end_peer(&tag);
                        continue;
                    }
                    let both = self.peers.get_mut(&tag).map(|p| {
                        p.verified_me = true;
                        p.verified_peer
                    });
                    self.send_app(&tag, lane::CTRL, json!({"t": "verified"}));
                    if both == Some(true) {
                        self.emit(Event::Verified { peer: tag_hex(&tag) });
                    }
                }
            }
            Command::Manual(code) => {
                if let Some((ch, env)) = self.rdv.open_manual(&code) {
                    self.pending_manual.push((ch, env));
                } else {
                    self.emit(Event::Notice("not a hush code (or not for this link)".into()));
                }
            }
            Command::End => self.end(EndReason::Ended),
        }
    }

    fn end_peer(&mut self, tag: &PeerTag) {
        if let Some(p) = self.peers.get_mut(tag)
            && let Some(bye) = p.session.close()
        {
            self.send_frame(tag, lane::CTRL, bye);
        }
        self.drop_peer(tag, false);
    }

    fn end(&mut self, reason: EndReason) {
        if self.ended {
            return;
        }
        let tags: Vec<PeerTag> = self.peers.keys().copied().collect();
        for tag in tags {
            if let Some(p) = self.peers.get_mut(&tag)
                && let Some(bye) = p.session.close()
            {
                self.send_frame(&tag, lane::CTRL, bye);
            }
        }
        for (_, (_, _, rtc, _)) in self.awaiting.drain() {
            let _ = rtc.send(RtcCmd::Close);
        }
        self.ended = true;
        self.end_reason = Some(reason);
        self.emit(Event::Ended);
    }
}

/// Keeps only a safe base name.
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = clean_name(base)
        .chars()
        .filter(|c| !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    // No leading dot (hidden files), no trailing dot or space (Windows drops them).
    let clean = clean.trim_start_matches('.').trim_end_matches(['.', ' ']).to_string();
    if clean.is_empty() {
        return "file".into();
    }
    // Windows device names open the device, not a file, whatever the extension.
    let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved { format!("_{clean}") } else { clean }
}

/// Creates a new file for `name` in `dir`, as `name (1)`, `name (2)`… when taken. Never opens an
/// existing path (nor follows a symlink planted there), so a download cannot overwrite anything.
fn create_unique(dir: &std::path::Path, name: &str) -> std::io::Result<(std::fs::File, PathBuf)> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 0..10_000 {
        let path = if n == 0 {
            dir.join(name)
        } else {
            dir.join(format!("{stem} ({n}){ext}"))
        };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free file name"))
}

fn guess_mime(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("txt" | "md") => "text/plain",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Al\u{202e}ice\u{0007} "), "Alice");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name(".bashrc"), "bashrc");
        assert_eq!(sanitize_file_name("a:b?.txt"), "ab.txt");
        assert_eq!(sanitize_file_name(""), "file");
        assert_eq!(sanitize_file_name("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_file_name("COM1"), "_COM1");
        assert_eq!(sanitize_file_name("report. . "), "report");
    }

    #[test]
    fn unique_files() {
        let dir = std::env::temp_dir().join(format!("hush-test-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (_, a) = create_unique(&dir, "x.txt").unwrap();
        assert_eq!(a.file_name().unwrap(), "x.txt");
        assert_eq!(
            create_unique(&dir, "x.txt").unwrap().1.file_name().unwrap(),
            "x (1).txt"
        );
        // A symlink planted under the name is not followed.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("target"), dir.join("y.txt")).unwrap();
            assert_eq!(
                create_unique(&dir, "y.txt").unwrap().1.file_name().unwrap(),
                "y (1).txt"
            );
            assert!(!dir.join("target").exists());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
