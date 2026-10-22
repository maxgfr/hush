//! The engine: every conversation of one person, over every transport, as one actor.
//!
//! It does what `web/src/app/{conversation,chat,peer,contacts,store}.ts` do in the browser:
//! - link handshakes over signaling (one-to-one, or a full mesh for a group);
//! - one ratcheting session per peer, carried by a nearby link, WebRTC, or the relays;
//! - the application messages (`PROTOCOL.md` §4): text, files, deletions, timers, verification,
//!   locking, contact cards;
//! - saved contacts, identifiers and the vault (`contacts.rs`, `store.rs`).
//!
//! The CLI drives one link conversation; the mobile apps drive everything, and provide the
//! Bluetooth and Wi-Fi links (`local.rs`).

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use hush_core::bundle::Bundle;
use hush_core::envelope::{BROADCAST, Envelope, PeerTag, kind};
use hush_core::handshake::{Established, LinkCreator, LinkCreatorPending, LinkJoiner, LinkJoinerReady};
use hush_core::link::{Link, LinkKeys, Mode};
use hush_core::session::{Session, lane};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

use crate::api::{
    ChatId, ChatKind, ChatStatus, Config, EndReason, Event, LocalHost, LocalKind, LocalMedium, Notice, PathKind,
};
use crate::contacts;
use crate::local::{self, LocalLink};
use crate::proto::{self, ChatMsg, Ctrl, FileMeta};
use crate::rtc::{self, RtcCmd, RtcEvent, RtcOptions};
use crate::signal::{Channel, ChannelId, LinkId, Rendezvous, Via};
use crate::store::{HistoryEntry, Store};
use crate::text::{clean_name, clean_text, create_unique, guess_mime, sanitize_file_name};
use crate::util::{new_id, now_ms, rng};

const TICK: Duration = Duration::from_millis(500);
const RESEND: Duration = Duration::from_secs(6);
const DIRECT_TIMEOUT: Duration = Duration::from_secs(20);
const IN_FLIGHT: usize = 64;
pub const MAX_GROUP: usize = 16;
/// Handshakes in flight at once, and how long one may wait for its next message.
const MAX_PENDING: usize = 4;
pub(crate) const PENDING_TIMEOUT: Duration = Duration::from_secs(45);
/// Images that download by themselves, from verified peers, a few at a time.
const AUTO_DOWNLOAD: u64 = 8 * 1024 * 1024;
const AUTO_DOWNLOADS: usize = 3;
/// Files sent to one peer at once.
const MAX_UPLOADS: usize = 8;
/// Frames held for a peer until a path works.
const MAX_QUEUED_FRAMES: usize = 256;

pub(crate) type RtcTx = mpsc::UnboundedSender<RtcCmd>;
pub(crate) type RtcKey = (ChatId, PeerTag);
pub type Reply<T> = oneshot::Sender<Result<T>>;

/// Closes a WebRTC link once the frames queued on it (a farewell) had time to leave.
fn close_later(rtc: RtcTx) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = rtc.send(RtcCmd::Close);
    });
}

pub fn tag_hex(t: &PeerTag) -> String {
    hex::encode(t)
}

fn parse_tag(s: &str) -> Option<PeerTag> {
    hex::decode(s).ok()?.try_into().ok()
}

// ── commands ────────────────────────────────────────────────────────────────

/// What the apps ask. Commands with a `reply` answer once done.
pub enum Cmd {
    CreateLink {
        group: bool,
        ttl_min: u64,
        passphrase: Option<String>,
        reply: Reply<(ChatId, String)>,
    },
    JoinLink {
        input: String,
        passphrase: Option<String>,
        reply: Reply<ChatId>,
    },
    Send {
        chat: ChatId,
        text: String,
    },
    SendFile {
        chat: ChatId,
        path: PathBuf,
    },
    GetFile {
        chat: ChatId,
        id: String,
    },
    CancelFile {
        chat: ChatId,
        id: String,
    },
    Delete {
        chat: ChatId,
        id: String,
    },
    /// The emojis match (`ok`) or not, for one peer or (`None`) every peer of the chat.
    Verify {
        chat: ChatId,
        peer: Option<String>,
        ok: bool,
    },
    Lock {
        chat: ChatId,
        locked: bool,
    },
    End {
        chat: ChatId,
    },
    /// A hand-carried code (`hushcode:…`).
    Manual {
        code: String,
    },
    /// Show handshake messages as codes too (no network).
    SetManual(bool),
    SaveContact {
        chat: ChatId,
        peer: String,
    },
    SetName(String),
    SetTtl(u64),
    SetLocal(bool),
    SetNetwork {
        signal: crate::signal::SignalConfig,
        ice_servers: Vec<crate::rtc::IceServer>,
        relay_only: bool,
    },
    Contacts(contacts::Cmd),
    Shutdown {
        reply: Reply<()>,
    },
}

/// What the app's nearby links report.
pub enum LocalIn {
    /// A link opened. `token` is the one we found the device by, when we dialed it.
    Opened {
        link: LinkId,
        kind: LocalKind,
        medium: LocalMedium,
        token: Option<Vec<u8>>,
    },
    Data {
        link: LinkId,
        data: Vec<u8>,
    },
    /// The app wrote this many bytes of what we gave it.
    Sent {
        link: LinkId,
        bytes: usize,
    },
    Closed {
        link: LinkId,
    },
}

pub(crate) enum Input {
    Cmd(Cmd),
    Local(LocalIn),
}

/// Results of work done off the actor (WebRTC descriptions, proofs of work).
pub(crate) enum Internal {
    /// A WebRTC offer (creator side of a link handshake, responder of a contact handshake).
    Offer {
        key: RtcKey,
        result: Result<String>,
    },
    /// A WebRTC answer (joiner side, initiator of a contact handshake).
    Answer {
        key: RtcKey,
        result: Result<String>,
    },
    Pow {
        key: String,
        nonce: u64,
    },
}

/// A handle on a running engine.
#[derive(Clone)]
pub struct App {
    tx: mpsc::UnboundedSender<Input>,
}

impl App {
    /// Starts an engine on the current Tokio runtime.
    pub fn start(cfg: Config, events: mpsc::UnboundedSender<Event>, host: Option<Arc<dyn LocalHost>>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let engine = Engine::new(cfg, events, host);
        tokio::spawn(engine.run(rx));
        Self { tx }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(Input::Cmd(cmd));
    }

    pub fn local(&self, ev: LocalIn) {
        let _ = self.tx.send(Input::Local(ev));
    }

    /// Sends a command built around a reply channel and waits for the answer.
    pub async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Cmd) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.send(make(tx));
        rx.await.map_err(|_| anyhow!("the engine stopped"))?
    }
}

// ── state ───────────────────────────────────────────────────────────────────

pub(crate) struct OutFile {
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
    meta: FileMeta,
    got: u64,
    next: u32,
    hasher: Sha256,
    file: Option<(std::fs::File, PathBuf)>,
}

pub(crate) struct Peer {
    pub session: Session,
    pub name: String,
    pub rtc: Option<RtcTx>,
    rtc_open: bool,
    pub local: Option<LinkId>,
    relayed: bool,
    signal: ChannelId,
    since: Instant,
    pub verified_me: bool,
    pub verified_peer: bool,
    uploads: Vec<Upload>,
    in_flight: usize,
    offers: HashMap<String, FileMeta>,
    downloads: HashMap<String, Download>,
    /// Downloads asked for before a path that carries files opened.
    wanted: Vec<String>,
    path: PathKind,
    /// The peer's verified contact card, kept until both sides compared the emojis.
    pub card: Option<Bundle>,
    /// We asked to save each other: store their card as soon as it is here.
    pub saving: bool,
    /// The contact secret, read as soon as it might be needed (the session can end any time).
    pub secret: Option<[u8; 32]>,
    pub offered: bool,
    /// Frames sealed before any path worked: sent once one does. Relayed right away, they could
    /// reach a peer not listening on the session's channel yet (it finishes the handshake last).
    queued: Vec<(u8, Vec<u8>)>,
}

impl Peer {
    pub fn verified(&self) -> bool {
        self.verified_me && self.verified_peer
    }
}

pub(crate) struct Awaiting {
    creator: LinkCreator,
    name: String,
    rtc: Option<RtcTx>,
    since: Instant,
}

pub(crate) struct LinkConv {
    keys: LinkKeys,
    pub channel: ChannelId,
    creator: bool,
    joiner: Option<LinkJoiner>,
    hs1: Option<Vec<u8>>,
    last_hs1: Instant,
    creating: HashMap<PeerTag, (LinkCreatorPending, String, RtcTx, Instant)>,
    awaiting: HashMap<PeerTag, Awaiting>,
    joining: HashMap<PeerTag, (LinkJoinerReady, String, RtcTx, Instant)>,
    answered: HashSet<PeerTag>,
    expires_ms: u64,
}

pub(crate) struct Chat {
    pub kind: ChatKind,
    pub group: bool,
    status: ChatStatus,
    pub link: Option<LinkConv>,
    pub peers: HashMap<PeerTag, Peer>,
    locked: bool,
    outgoing: HashMap<String, OutFile>,
    /// Who wrote each message and file (`None`: us). An id is its author's: a peer cannot take
    /// one already used, nor delete someone else's.
    authors: HashMap<String, Option<PeerTag>>,
    /// When offered files disappear.
    expiry: HashMap<String, u64>,
    /// Where received files were saved.
    received: HashMap<String, PathBuf>,
    pub ended: bool,
}

impl Chat {
    pub fn new(kind: ChatKind, group: bool) -> Self {
        Self {
            kind,
            group,
            status: ChatStatus::Connecting,
            link: None,
            peers: HashMap::new(),
            locked: false,
            outgoing: HashMap::new(),
            authors: HashMap::new(),
            expiry: HashMap::new(),
            received: HashMap::new(),
            ended: false,
        }
    }

    pub fn contact_key(&self) -> Option<&str> {
        match &self.kind {
            ChatKind::Contact { key } => Some(key),
            _ => None,
        }
    }

    pub fn connected(&self) -> bool {
        !self.ended && !self.peers.is_empty()
    }
}

pub(crate) struct Engine {
    pub(crate) cfg: Config,
    events: mpsc::UnboundedSender<Event>,
    host: Option<Arc<dyn LocalHost>>,
    pub(crate) rdv: Rendezvous,
    local_out: mpsc::UnboundedReceiver<(LinkId, Vec<u8>)>,
    rtc_tx: mpsc::UnboundedSender<(RtcKey, RtcEvent)>,
    rtc_rx: mpsc::UnboundedReceiver<(RtcKey, RtcEvent)>,
    pub(crate) internal_tx: mpsc::UnboundedSender<Internal>,
    internal_rx: mpsc::UnboundedReceiver<Internal>,
    pub(crate) chats: HashMap<ChatId, Chat>,
    next_chat: ChatId,
    /// Link channels, and per-peer session channels (relay fallback).
    link_channels: HashMap<ChannelId, ChatId>,
    session_channels: HashMap<ChannelId, (ChatId, PeerTag)>,
    links: HashMap<LinkId, LocalLink>,
    advertised: Vec<Vec<u8>>,
    scanned: Vec<Vec<u8>>,
    pub(crate) store: Store,
    pub(crate) contacts: contacts::State,
    stopping: Option<Reply<()>>,
}

impl Engine {
    pub(crate) fn new(cfg: Config, events: mpsc::UnboundedSender<Event>, host: Option<Arc<dyn LocalHost>>) -> Self {
        let (local_tx, local_out) = mpsc::unbounded_channel();
        let (rtc_tx, rtc_rx) = mpsc::unbounded_channel();
        let (internal_tx, internal_rx) = mpsc::unbounded_channel();
        let rdv = Rendezvous::new(&cfg.signal, local_tx);
        let store = Store::new(cfg.data_dir.clone());
        Self {
            cfg,
            events,
            host,
            rdv,
            local_out,
            rtc_tx,
            rtc_rx,
            internal_tx,
            internal_rx,
            chats: HashMap::new(),
            next_chat: 1,
            link_channels: HashMap::new(),
            session_channels: HashMap::new(),
            links: HashMap::new(),
            advertised: Vec::new(),
            scanned: Vec::new(),
            store,
            contacts: contacts::State::default(),
            stopping: None,
        }
    }

    pub(crate) fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    pub(crate) fn name(&self) -> String {
        self.cfg.name.clone()
    }

    pub(crate) fn rtc_sender(&self) -> mpsc::UnboundedSender<(RtcKey, RtcEvent)> {
        self.rtc_tx.clone()
    }

    pub(crate) fn rtc_opts(&self) -> RtcOptions {
        RtcOptions {
            ice_servers: self.cfg.ice_servers.clone(),
            relay_only: self.cfg.relay_only,
        }
    }

    /// Runs until `Shutdown` (or every handle is dropped).
    pub(crate) async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Input>) {
        self.emit(Event::Vault(self.store.state()));
        let mut tick = tokio::time::interval(TICK);
        loop {
            tokio::select! {
                input = rx.recv() => match input {
                    None => break,
                    Some(Input::Cmd(c)) => self.on_command(c).await,
                    Some(Input::Local(l)) => self.on_local(l),
                },
                env = self.rdv.next() => {
                    let Some((ch, env)) = env else { break };
                    self.on_envelope(ch, env, Via::Network);
                }
                ev = self.rtc_rx.recv() => {
                    if let Some((key, ev)) = ev { self.on_rtc(key, ev); }
                }
                out = self.local_out.recv() => {
                    if let Some((link, payload)) = out { self.local_write_envelope(link, &payload); }
                }
                int = self.internal_rx.recv() => {
                    if let Some(i) = int { self.on_internal(i); }
                }
                _ = tick.tick() => self.on_tick(),
            }
            if let Some(reply) = self.stopping.take() {
                let _ = reply.send(Ok(()));
                break;
            }
        }
        let ids: Vec<ChatId> = self.chats.keys().copied().collect();
        for id in ids {
            self.end_chat(id, EndReason::Ended);
        }
        // Give farewells a moment to leave.
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    async fn on_command(&mut self, c: Cmd) {
        match c {
            Cmd::CreateLink {
                group,
                ttl_min,
                passphrase,
                reply,
            } => {
                let _ = reply.send(self.create_link(group, ttl_min, passphrase));
            }
            Cmd::JoinLink {
                input,
                passphrase,
                reply,
            } => {
                let _ = reply.send(self.join_link(&input, passphrase));
            }
            Cmd::Send { chat, text } => self.send_text(chat, &text),
            Cmd::SendFile { chat, path } => {
                if let Err(e) = self.offer_file(chat, path) {
                    self.emit(Event::Notice {
                        chat,
                        notice: Notice::CannotSend { reason: e.to_string() },
                    });
                }
            }
            Cmd::GetFile { chat, id } => self.request_file(chat, &id),
            Cmd::CancelFile { chat, id } => self.cancel_file(chat, &id),
            Cmd::Delete { chat, id } => self.delete_message(chat, &id),
            Cmd::Verify { chat, peer, ok } => self.verify(chat, peer.as_deref(), ok),
            Cmd::Lock { chat, locked } => self.set_locked(chat, locked),
            Cmd::End { chat } => self.end_chat(chat, EndReason::Ended),
            Cmd::Manual { code } => match self.rdv.open_manual(&code) {
                Some((ch, env)) => self.on_envelope(ch, env, Via::Manual),
                None => {
                    if let Some(chat) = self.chats.keys().next().copied() {
                        self.emit(Event::Notice {
                            chat,
                            notice: Notice::NotACode,
                        });
                    }
                }
            },
            Cmd::SetManual(on) => {
                self.rdv.manual = on;
                let resend: Vec<(ChatId, Vec<u8>)> = self
                    .chats
                    .iter()
                    .filter_map(|(id, c)| c.link.as_ref()?.hs1.clone().map(|h| (*id, h)))
                    .collect();
                if on {
                    for (chat, hs1) in resend {
                        self.send_hs1(chat, &hs1);
                    }
                }
            }
            Cmd::SaveContact { chat, peer } => {
                if let Some(tag) = parse_tag(&peer) {
                    self.save_contact(chat, tag);
                }
            }
            Cmd::SetName(name) => {
                self.cfg.name = clean_name(&name);
                let n = self.cfg.name.clone();
                let all: Vec<(ChatId, PeerTag)> = self
                    .chats
                    .iter()
                    .flat_map(|(id, c)| c.peers.keys().map(move |t| (*id, *t)))
                    .collect();
                for (chat, tag) in all {
                    self.send_app(chat, &tag, lane::CTRL, json!({"t": "name", "name": n}));
                }
            }
            Cmd::SetTtl(ttl) => self.cfg.ttl = ttl.min(proto::MAX_TTL),
            Cmd::SetLocal(on) => {
                self.cfg.local = on;
                if !on {
                    let links: Vec<LinkId> = self.links.keys().copied().collect();
                    for l in links {
                        self.close_link(l);
                    }
                }
                self.refresh_discovery();
            }
            Cmd::SetNetwork {
                signal,
                ice_servers,
                relay_only,
            } => {
                self.cfg.ice_servers = ice_servers;
                self.cfg.relay_only = relay_only;
                self.rdv.reconfigure(&signal);
                self.cfg.signal = signal;
            }
            Cmd::Contacts(c) => self.on_contacts_command(c).await,
            Cmd::Shutdown { reply } => self.stopping = Some(reply),
        }
    }

    fn on_internal(&mut self, i: Internal) {
        match i {
            Internal::Offer { key, result } => {
                if self.contacts_owns(&key) {
                    self.contact_offer_ready(key, result);
                } else {
                    self.link_offer_ready(key, result);
                }
            }
            Internal::Answer { key, result } => {
                if self.contacts_owns(&key) {
                    self.contact_answer_ready(key, result);
                } else {
                    self.link_answer_ready(key, result);
                }
            }
            Internal::Pow { key, nonce } => self.request_pow_ready(&key, nonce),
        }
    }

    fn on_tick(&mut self) {
        self.rdv.rotate();
        let now = now_ms();
        let ids: Vec<ChatId> = self.chats.keys().copied().collect();
        for id in ids {
            self.tick_link(id, now);
            let tags: Vec<PeerTag> = self
                .chats
                .get(&id)
                .map(|c| c.peers.keys().copied().collect())
                .unwrap_or_default();
            for tag in tags {
                let mut out = Vec::new();
                let mut changed = false;
                if let Some(p) = self.chats.get_mut(&id).and_then(|c| c.peers.get_mut(&tag)) {
                    out = p.session.poll(now, &mut rng());
                    if p.local.is_none() && !p.rtc_open && !p.relayed && p.since.elapsed() >= DIRECT_TIMEOUT {
                        p.relayed = true;
                        changed = true;
                    }
                }
                for (l, f) in out {
                    self.send_frame(id, &tag, l, f);
                }
                if changed {
                    self.refresh_path(id, &tag);
                    self.fail_wanted(id, &tag);
                }
                self.pump(id, &tag);
            }
        }
        if self.cfg.discard_files {
            let gone: Vec<(ChatId, String)> = self
                .chats
                .iter()
                .flat_map(|(chat, c)| {
                    c.expiry
                        .iter()
                        .filter(|(_, at)| **at <= now)
                        .map(|(id, _)| (*chat, id.clone()))
                })
                .collect();
            for (chat, id) in gone {
                if let Some(c) = self.chats.get_mut(&chat) {
                    c.expiry.remove(&id);
                }
                self.discard_received(chat, &id);
            }
        }
        self.contacts_tick();
        let idle: Vec<LinkId> = self.links.iter().filter(|(_, l)| l.idle()).map(|(id, _)| *id).collect();
        for link in idle {
            self.close_link(link);
        }
        self.refresh_discovery();
        let ended: Vec<ChatId> = self.chats.iter().filter(|(_, c)| c.ended).map(|(id, _)| *id).collect();
        for id in ended {
            self.chats.remove(&id);
        }
    }

    // ── conversations ───────────────────────────────────────────────────────

    pub(crate) fn new_chat(&mut self, chat: Chat) -> ChatId {
        let id = self.next_chat;
        self.next_chat += 1;
        self.chats.insert(id, chat);
        id
    }

    fn create_link(&mut self, group: bool, ttl_min: u64, passphrase: Option<String>) -> Result<(ChatId, String)> {
        let pass = passphrase.filter(|p| !p.is_empty());
        let mode = if group { Mode::Group } else { Mode::Direct };
        let link = Link::generate(
            mode,
            now_ms(),
            ttl_min.clamp(1, 7 * 24 * 60),
            pass.is_some(),
            &mut rng(),
        );
        let keys = link.keys(pass.as_deref()).map_err(|e| anyhow!("{e}"))?;
        let url = link.url(&self.cfg.base);
        let id = self.open_link_chat(keys, true, link.expires_min * 60_000, Some(url.clone()));
        Ok((id, url))
    }

    fn join_link(&mut self, input: &str, passphrase: Option<String>) -> Result<ChatId> {
        let link = Link::parse(input).map_err(crate::util::link_error)?;
        if link.is_expired(now_ms()) {
            bail!("this link has expired");
        }
        let pass = passphrase.filter(|p| !p.is_empty());
        if link.passphrase && pass.is_none() {
            bail!("this link needs a passphrase");
        }
        let keys = link
            .keys(if link.passphrase { pass.as_deref() } else { None })
            .map_err(|_| anyhow!("bad passphrase"))?;
        let id = self.open_link_chat(keys, false, link.expires_min * 60_000, None);
        let (state, hs1) = LinkJoiner::start(
            &self.chats[&id].link.as_ref().expect("link chat").keys,
            &proto::hello(&self.cfg.name, None),
            &mut rng(),
        );
        if let Some(l) = self.chats.get_mut(&id).and_then(|c| c.link.as_mut()) {
            l.joiner = Some(state);
            l.hs1 = Some(hs1.clone());
        }
        self.send_hs1(id, &hs1);
        Ok(id)
    }

    fn open_link_chat(&mut self, keys: LinkKeys, creator: bool, expires_ms: u64, url: Option<String>) -> ChatId {
        let group = keys.mode == Mode::Group;
        let topic_keys = keys.clone();
        let channel = self.rdv.add(Channel::new(*keys.env_key, move |d| topic_keys.topic(d)));
        let mut chat = Chat::new(ChatKind::Link { group, creator }, group);
        chat.status = ChatStatus::Waiting;
        chat.link = Some(LinkConv {
            keys,
            channel,
            creator,
            joiner: None,
            hs1: None,
            last_hs1: Instant::now(),
            creating: HashMap::new(),
            awaiting: HashMap::new(),
            joining: HashMap::new(),
            answered: HashSet::new(),
            expires_ms,
        });
        let id = self.new_chat(chat);
        self.link_channels.insert(channel, id);
        self.emit(Event::ChatOpened {
            chat: id,
            kind: ChatKind::Link { group, creator },
            url,
            expires_ms: Some(expires_ms),
        });
        self.emit(Event::ChatStatus {
            chat: id,
            status: ChatStatus::Waiting,
        });
        self.refresh_discovery();
        id
    }

    fn link_channel(&self, chat: ChatId) -> Option<ChannelId> {
        Some(self.chats.get(&chat)?.link.as_ref()?.channel)
    }

    fn send_hs1(&mut self, chat: ChatId, hs1: &[u8]) {
        let Some(ch) = self.link_channel(chat) else { return };
        if let Some(l) = self.chats.get_mut(&chat).and_then(|c| c.link.as_mut()) {
            l.last_hs1 = Instant::now();
        }
        if let Some(code) = self.rdv.send(ch, kind::HS1, hs1, None)
            && self.rdv.manual
        {
            self.emit(Event::Code { chat, code });
        }
    }

    fn send_link(&self, chat: ChatId, kind: u8, body: &[u8], to: PeerTag) {
        let Some(ch) = self.link_channel(chat) else { return };
        if let Some(code) = self.rdv.send(ch, kind, body, Some(to))
            && self.rdv.manual
            && matches!(kind, kind::HS2 | kind::HS3)
        {
            self.emit(Event::Code { chat, code });
        }
    }

    fn tick_link(&mut self, id: ChatId, now: u64) {
        let Some(c) = self.chats.get_mut(&id) else { return };
        let Some(l) = c.link.as_mut() else { return };
        let mut closed = Vec::new();
        l.awaiting.retain(|_, a| {
            let alive = a.since.elapsed() < PENDING_TIMEOUT;
            if !alive && let Some(r) = a.rtc.take() {
                closed.push(r);
            }
            alive
        });
        l.creating.retain(|_, (_, _, r, since)| {
            let alive = since.elapsed() < PENDING_TIMEOUT;
            if !alive {
                closed.push(r.clone());
            }
            alive
        });
        l.joining.retain(|_, (_, _, r, since)| {
            let alive = since.elapsed() < PENDING_TIMEOUT;
            if !alive {
                closed.push(r.clone());
            }
            alive
        });
        for r in closed {
            let _ = r.send(RtcCmd::Close);
        }
        let waiting = c.peers.is_empty();
        if waiting && now >= l.expires_ms && l.awaiting.is_empty() && l.creating.is_empty() && l.joining.is_empty() {
            self.end_chat(id, EndReason::Expired);
            return;
        }
        let resend = waiting && l.hs1.is_some() && l.last_hs1.elapsed() >= RESEND && !self.rdv.manual;
        if resend {
            let hs1 = l.hs1.clone().expect("checked");
            self.send_hs1(id, &hs1);
        }
    }

    pub(crate) fn set_status(&mut self, chat: ChatId, status: ChatStatus) {
        if let Some(c) = self.chats.get_mut(&chat)
            && c.status != status
            && !c.ended
        {
            c.status = status;
            self.emit(Event::ChatStatus { chat, status });
        }
    }

    // ── link handshake ──────────────────────────────────────────────────────

    pub(crate) fn on_envelope(&mut self, ch: ChannelId, env: Envelope, via: Via) {
        if let Some(&(chat, tag)) = self.session_channels.get(&ch) {
            if env.kind == kind::SESSION {
                self.on_frame(chat, &tag, &env.body, via.link());
            }
            return;
        }
        if let Some(&chat) = self.link_channels.get(&ch) {
            let to_us = env.to != BROADCAST;
            match env.kind {
                kind::HS1 => self.on_hs1(chat, env.from, &env.body, via.link()),
                kind::HS2 if to_us => self.on_hs2(chat, env.from, &env.body, via.link()),
                kind::HS3 if to_us => self.on_hs3(chat, env.from, &env.body),
                kind::TAKEN if to_us => {
                    let direct = self.chats.get(&chat).is_some_and(|c| !c.group && c.peers.is_empty());
                    if direct {
                        self.end_chat(chat, EndReason::Taken);
                    }
                }
                _ => {}
            }
            return;
        }
        self.on_contact_envelope(ch, env, via);
    }

    fn is_member(c: &Chat) -> bool {
        c.link.as_ref().is_some_and(|l| l.creator) || !c.peers.is_empty()
    }

    /// `local`: the nearby link this HS1 came over. Only then is the session kept off the
    /// network: a peer merely heard on a link earlier may be someone replaying its envelopes.
    fn on_hs1(&mut self, chat: ChatId, from: PeerTag, hs1: &[u8], local: Option<LinkId>) {
        let Some(c) = self.chats.get(&chat) else { return };
        let Some(l) = c.link.as_ref() else { return };
        if !Self::is_member(c) || l.answered.contains(&from) || c.peers.contains_key(&from) {
            return;
        }
        let pending = l.creating.len() + l.awaiting.len();
        let full = if c.group {
            // Someone holding the link could otherwise open connections without end.
            c.locked || c.peers.len() >= MAX_GROUP - 1 || pending >= MAX_PENDING
        } else {
            !c.peers.is_empty() || pending > 0
        };
        if full {
            if !c.group {
                self.send_link(chat, kind::TAKEN, &[], from);
            }
            return;
        }
        let Ok((state, payload)) = LinkCreatorPending::read_hs1(&l.keys, hs1) else {
            return; // not someone holding this link (or a wrong passphrase)
        };
        let name = proto::parse_hello(&payload).map(|h| h.name).unwrap_or_default();
        self.set_status(chat, ChatStatus::Connecting);
        let (opts, rtc_tx) = (self.rtc_opts(), self.rtc_tx.clone());
        let l = self
            .chats
            .get_mut(&chat)
            .and_then(|c| c.link.as_mut())
            .expect("checked");
        l.answered.insert(from);
        if local.is_some() {
            // A nearby link: no WebRTC, the session travels on it.
            let Ok((creator, hs2)) = state.write_hs2(&proto::hello(&self.cfg.name, None), &mut rng()) else {
                return;
            };
            l.awaiting.insert(
                from,
                Awaiting {
                    creator,
                    name,
                    rtc: None,
                    since: Instant::now(),
                },
            );
            self.send_link(chat, kind::HS2, &hs2, from);
            return;
        }
        let rtc = rtc::spawn((chat, from), opts, rtc_tx);
        l.creating.insert(from, (state, name, rtc.clone(), Instant::now()));
        let (tx, rx) = oneshot::channel();
        let _ = rtc.send(RtcCmd::CreateOffer(tx));
        let internal = self.internal_tx.clone();
        tokio::spawn(async move {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("closed")));
            let _ = internal.send(Internal::Offer {
                key: (chat, from),
                result,
            });
        });
    }

    fn link_offer_ready(&mut self, (chat, from): RtcKey, result: Result<String>) {
        let Some(l) = self.chats.get_mut(&chat).and_then(|c| c.link.as_mut()) else {
            return;
        };
        let Some((state, name, rtc, _)) = l.creating.remove(&from) else {
            return;
        };
        let written = result.and_then(|offer| {
            state
                .write_hs2(&proto::hello(&self.cfg.name, Some(&offer)), &mut rng())
                .map_err(|e| anyhow!("{e}"))
        });
        let Ok((creator, hs2)) = written else {
            let _ = rtc.send(RtcCmd::Close);
            return;
        };
        l.awaiting.insert(
            from,
            Awaiting {
                creator,
                name,
                rtc: Some(rtc),
                since: Instant::now(),
            },
        );
        self.send_link(chat, kind::HS2, &hs2, from);
    }

    fn on_hs3(&mut self, chat: ChatId, from: PeerTag, hs3: &[u8]) {
        let Some(a) = self
            .chats
            .get_mut(&chat)
            .and_then(|c| c.link.as_mut())
            .and_then(|l| l.awaiting.remove(&from))
        else {
            return;
        };
        let Ok((payload, est)) = a.creator.read_hs3(hs3) else {
            if let Some(r) = a.rtc {
                let _ = r.send(RtcCmd::Close);
            }
            return;
        };
        if let (Some(r), Some(sdp)) = (&a.rtc, proto::parse_hello(&payload).and_then(|h| h.sdp)) {
            let _ = r.send(RtcCmd::AcceptAnswer(sdp));
        }
        let ch = self.link_channel(chat);
        self.add_peer(chat, from, &est, a.name, a.rtc, ch, false);
    }

    fn on_hs2(&mut self, chat: ChatId, from: PeerTag, hs2: &[u8], via: Option<LinkId>) {
        let ch = self.link_channel(chat);
        let local = via.or_else(|| ch.and_then(|ch| self.rdv.local_peer(ch, &from)));
        let Some(c) = self.chats.get(&chat) else { return };
        let Some(l) = c.link.as_ref() else { return };
        // A one-to-one link answers one member: a second HS2 (someone else holding the link,
        // racing the creator's) would add a hidden peer to the conversation.
        if l.answered.contains(&from)
            || (!c.group && (!c.peers.is_empty() || !l.joining.is_empty()))
            || c.peers.len() + l.joining.len() >= MAX_GROUP - 1
            || l.joining.len() >= MAX_PENDING
        {
            return;
        }
        let Some(joiner) = &l.joiner else { return };
        let Ok((ready, payload)) = joiner.read_hs2(hs2) else {
            return;
        };
        let hello = proto::parse_hello(&payload).unwrap_or_default();
        self.set_status(chat, ChatStatus::Connecting);
        let (opts, rtc_tx) = (self.rtc_opts(), self.rtc_tx.clone());
        let l = self
            .chats
            .get_mut(&chat)
            .and_then(|c| c.link.as_mut())
            .expect("checked");
        l.answered.insert(from);
        match (hello.sdp, local) {
            (None, Some(_)) => {
                // The member answered over a nearby link: no WebRTC.
                let (hs3, est) = ready.write_hs3(&proto::hello(&self.cfg.name, None));
                self.send_link(chat, kind::HS3, &hs3, from);
                self.add_peer(chat, from, &est, hello.name, None, ch, false);
            }
            (Some(sdp), _) => {
                let rtc = rtc::spawn((chat, from), opts, rtc_tx);
                l.joining.insert(from, (ready, hello.name, rtc.clone(), Instant::now()));
                let (tx, rx) = oneshot::channel();
                let _ = rtc.send(RtcCmd::AcceptOffer(sdp, tx));
                let internal = self.internal_tx.clone();
                tokio::spawn(async move {
                    let result = rx.await.unwrap_or_else(|_| Err(anyhow!("closed")));
                    let _ = internal.send(Internal::Answer {
                        key: (chat, from),
                        result,
                    });
                });
            }
            (None, None) => {}
        }
    }

    fn link_answer_ready(&mut self, (chat, from): RtcKey, result: Result<String>) {
        let Some((ready, name, rtc, _)) = self
            .chats
            .get_mut(&chat)
            .and_then(|c| c.link.as_mut())
            .and_then(|l| l.joining.remove(&from))
        else {
            return;
        };
        let Ok(answer) = result else {
            let _ = rtc.send(RtcCmd::Close);
            return;
        };
        let (hs3, est) = ready.write_hs3(json!({"v": 1, "sdp": answer}).to_string().as_bytes());
        self.send_link(chat, kind::HS3, &hs3, from);
        let ch = self.link_channel(chat);
        self.add_peer(chat, from, &est, name, Some(rtc), ch, false);
    }

    /// A finished handshake: the peer's session, on whichever path is there. `rdv_channel` is
    /// the channel the handshake ran on (to find a nearby link the peer used).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_peer(
        &mut self,
        chat: ChatId,
        tag: PeerTag,
        est: &Established,
        name: String,
        rtc: Option<RtcTx>,
        rdv_channel: Option<ChannelId>,
        verified: bool,
    ) {
        let Some(c) = self.chats.get(&chat) else {
            if let Some(r) = rtc {
                let _ = r.send(RtcCmd::Close);
            }
            return;
        };
        if c.ended {
            if let Some(r) = rtc {
                let _ = r.send(RtcCmd::Close);
            }
            return;
        }
        let name = self.unique_name(chat, &name, None);
        let session = Session::new(est, now_ms());
        let sk = session.signal_keys();
        let signal = self.rdv.add(Channel::new(sk.env_key(), move |d| sk.topic(d)));
        self.session_channels.insert(signal, (chat, tag));
        let local = rdv_channel.and_then(|ch| self.rdv.local_peer(ch, &tag));
        if let Some(l) = local.and_then(|l| self.links.get_mut(&l)) {
            l.peer = Some((chat, tag));
        }
        let sas = est.sas.map(|s| s.to_vec()).unwrap_or_default();
        let c = self.chats.get_mut(&chat).expect("checked");
        c.peers.insert(
            tag,
            Peer {
                session,
                name: name.clone(),
                rtc,
                rtc_open: false,
                local,
                relayed: false,
                signal,
                since: Instant::now(),
                verified_me: verified,
                verified_peer: verified,
                uploads: Vec::new(),
                in_flight: 0,
                offers: HashMap::new(),
                downloads: HashMap::new(),
                wanted: Vec::new(),
                path: PathKind::Connecting,
                card: None,
                saving: false,
                secret: None,
                offered: false,
                queued: Vec::new(),
            },
        );
        let group = c.group;
        self.set_status(chat, ChatStatus::Open);
        self.emit(Event::PeerJoined {
            chat,
            peer: tag_hex(&tag),
            name: name.clone(),
            sas,
            verified,
        });
        if group {
            self.emit(Event::Notice {
                chat,
                notice: Notice::Joined { name },
            });
        }
        self.refresh_path(chat, &tag);
        let n = self.cfg.name.clone();
        self.send_app(chat, &tag, lane::CTRL, json!({"t": "name", "name": n}));
        let locked = self.chats.get(&chat).is_some_and(|c| c.locked);
        if locked {
            self.send_app(chat, &tag, lane::CTRL, json!({"t": "lock", "locked": true}));
        }
        self.refresh_discovery();
    }

    /// A peer's name, made distinct from every other member's (and ours) in a group.
    fn unique_name(&self, chat: ChatId, name: &str, peer: Option<&PeerTag>) -> String {
        let Some(c) = self.chats.get(&chat) else {
            return name.to_string();
        };
        if !c.group || name.is_empty() {
            return name.to_string();
        }
        let taken: HashSet<String> = std::iter::once(self.cfg.name.to_lowercase())
            .chain(
                c.peers
                    .iter()
                    .filter(|(t, _)| Some(*t) != peer)
                    .map(|(_, p)| p.name.to_lowercase()),
            )
            .collect();
        if !taken.contains(&name.to_lowercase()) {
            return name.to_string();
        }
        (2..)
            .map(|n| format!("{name} ({n})"))
            .find(|n| !taken.contains(&n.to_lowercase()))
            .expect("a free name")
    }

    // ── paths and frames ────────────────────────────────────────────────────

    fn refresh_path(&mut self, chat: ChatId, tag: &PeerTag) {
        let Some(p) = self.chats.get(&chat).and_then(|c| c.peers.get(tag)) else {
            return;
        };
        let path = match p.local.and_then(|l| self.links.get(&l)) {
            Some(link) => PathKind::Local(link.medium),
            None if p.rtc_open => PathKind::Direct,
            None if p.relayed => PathKind::Relayed,
            None => PathKind::Connecting,
        };
        if path == p.path {
            return;
        }
        let queued = match self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) {
            Some(p) => {
                p.path = path;
                std::mem::take(&mut p.queued)
            }
            None => Vec::new(),
        };
        for (l, f) in queued {
            self.send_frame(chat, tag, l, f);
        }
        self.emit(Event::PeerPath {
            chat,
            peer: tag_hex(tag),
            path,
        });
        if matches!(path, PathKind::Local(_) | PathKind::Direct) {
            let wanted = self
                .chats
                .get_mut(&chat)
                .and_then(|c| c.peers.get_mut(tag))
                .map(|p| std::mem::take(&mut p.wanted))
                .unwrap_or_default();
            for id in wanted {
                self.request_file(chat, &id);
            }
            self.pump(chat, tag);
        }
    }

    fn on_rtc(&mut self, (chat, tag): RtcKey, ev: RtcEvent) {
        match ev {
            RtcEvent::Open => {
                if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(&tag)) {
                    p.rtc_open = true;
                }
                self.refresh_path(chat, &tag);
            }
            RtcEvent::Frame(frame) => self.on_frame(chat, &tag, &frame, None),
            RtcEvent::Closed => {
                if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(&tag)) {
                    p.rtc_open = false;
                    p.rtc = None;
                }
                self.refresh_path(chat, &tag);
            }
            RtcEvent::Sent => {
                if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(&tag)) {
                    p.in_flight = p.in_flight.saturating_sub(1);
                }
                self.pump(chat, &tag);
            }
        }
    }

    pub(crate) fn send_frame(&mut self, chat: ChatId, tag: &PeerTag, l: u8, frame: Vec<u8>) {
        let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
            return;
        };
        if p.path == PathKind::Connecting && p.queued.len() < MAX_QUEUED_FRAMES {
            p.queued.push((l, frame));
            return;
        }
        let p = &self.chats[&chat].peers[tag];
        if let Some(link) = p.local
            && self.links.contains_key(&link)
        {
            self.local_write(link, l, &frame);
        } else if p.rtc_open
            && let Some(r) = &p.rtc
        {
            let _ = r.send(RtcCmd::Send(usize::from(l), frame));
        } else if l != lane::FILE {
            self.rdv.send(p.signal, kind::SESSION, &frame, None);
        }
    }

    pub(crate) fn send_app(&mut self, chat: ChatId, tag: &PeerTag, l: u8, v: Value) {
        let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
            return;
        };
        let Ok(frame) = p.session.seal(l, v.to_string().as_bytes()) else {
            return;
        };
        self.send_frame(chat, tag, l, frame);
    }

    fn broadcast_app(&mut self, chat: ChatId, l: u8, v: &Value) {
        let tags: Vec<PeerTag> = self
            .chats
            .get(&chat)
            .map(|c| c.peers.keys().copied().collect())
            .unwrap_or_default();
        for t in tags {
            self.send_app(chat, &t, l, v.clone());
        }
    }

    /// A session frame from `tag`, over the nearby link `via` or else the network.
    fn on_frame(&mut self, chat: ChatId, tag: &PeerTag, frame: &[u8], via: Option<LinkId>) {
        let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
            return;
        };
        let Ok(opened) = p.session.open(frame, now_ms(), &mut rng()) else {
            return;
        };
        // A peer on a nearby link sends there. Its frames coming another way mean the link we
        // chose is not its own (someone nearby replayed its envelopes to draw our traffic): stop
        // sending there.
        if let Some(link) = p.local
            && via != Some(link)
        {
            p.local = None;
            if let Some(l) = self.links.get_mut(&link) {
                l.peer = None;
            }
            self.refresh_path(chat, tag);
        }
        for (l, f) in opened.replies {
            self.send_frame(chat, tag, l, f);
        }
        for (l, data) in opened.messages {
            self.on_message(chat, tag, l, &data);
        }
        if opened.closed {
            self.drop_peer(chat, tag, true);
        }
    }

    fn drop_peer(&mut self, chat: ChatId, tag: &PeerTag, by_peer: bool) {
        let Some(c) = self.chats.get_mut(&chat) else { return };
        let Some(p) = c.peers.remove(tag) else { return };
        let group = c.group;
        discard_partial(&p);
        if let Some(r) = p.rtc.clone() {
            close_later(r);
        }
        if let Some(l) = p.local.and_then(|l| self.links.get_mut(&l)) {
            l.peer = None;
        }
        self.rdv.remove(p.signal);
        self.session_channels.remove(&p.signal);
        self.emit(Event::PeerLeft {
            chat,
            peer: tag_hex(tag),
            name: p.name.clone(),
        });
        if group {
            self.emit(Event::Notice {
                chat,
                notice: Notice::Left { name: p.name },
            });
            let empty = self.chats.get(&chat).is_some_and(|c| c.peers.is_empty());
            if empty && !self.chats[&chat].link.as_ref().is_some_and(|l| l.creator) {
                self.set_status(chat, ChatStatus::Waiting);
            }
        } else {
            self.end_chat(chat, if by_peer { EndReason::PeerLeft } else { EndReason::Ended });
        }
    }

    pub(crate) fn end_chat(&mut self, chat: ChatId, reason: EndReason) {
        let Some(c) = self.chats.get(&chat) else { return };
        if c.ended {
            return;
        }
        let tags: Vec<PeerTag> = c.peers.keys().copied().collect();
        for tag in &tags {
            if let Some(bye) = self
                .chats
                .get_mut(&chat)
                .and_then(|c| c.peers.get_mut(tag))
                .and_then(|p| p.session.close())
            {
                self.send_frame(chat, tag, lane::CTRL, bye);
            }
        }
        let c = self.chats.get_mut(&chat).expect("checked");
        c.ended = true;
        let mut rtcs: Vec<RtcTx> = c.peers.values().filter_map(|p| p.rtc.clone()).collect();
        let signals: Vec<ChannelId> = c.peers.values().map(|p| p.signal).collect();
        let link_ch = c.link.as_ref().map(|l| l.channel);
        if let Some(l) = c.link.as_mut() {
            rtcs.extend(l.creating.drain().map(|(_, v)| v.2));
            rtcs.extend(l.awaiting.drain().filter_map(|(_, v)| v.rtc));
            rtcs.extend(l.joining.drain().map(|(_, v)| v.2));
        }
        for p in c.peers.values() {
            discard_partial(p);
        }
        c.peers.clear();
        if self.cfg.discard_files {
            for (_, path) in c.received.drain() {
                let _ = std::fs::remove_file(path);
            }
        }
        for r in rtcs {
            close_later(r);
        }
        for s in signals {
            self.rdv.remove(s);
            self.session_channels.remove(&s);
        }
        if let Some(ch) = link_ch {
            self.rdv.remove(ch);
            self.link_channels.remove(&ch);
        }
        for l in self.links.values_mut() {
            if l.peer.is_some_and(|(c, _)| c == chat) {
                l.peer = None;
            }
        }
        self.contacts_chat_ended(chat);
        self.emit(Event::ChatEnded { chat, reason });
        self.refresh_discovery();
    }

    // ── application messages ────────────────────────────────────────────────

    fn on_message(&mut self, chat: ChatId, tag: &PeerTag, l: u8, data: &[u8]) {
        match l {
            lane::FILE => self.on_chunk(chat, tag, data),
            lane::CTRL => {
                if let Some(m) = proto::parse_ctrl(data) {
                    self.on_ctrl(chat, tag, m);
                }
            }
            lane::CHAT => {
                if let Some(m) = proto::parse_chat(data) {
                    self.on_chat(chat, tag, m);
                }
            }
            _ => {}
        }
    }

    fn on_ctrl(&mut self, chat: ChatId, tag: &PeerTag, m: Ctrl) {
        let peer = tag_hex(tag);
        match m {
            Ctrl::Name(n) => {
                let n = self.unique_name(chat, &n, Some(tag));
                let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
                    return;
                };
                if n == p.name {
                    return;
                }
                let old = std::mem::replace(&mut p.name, n.clone());
                self.emit(Event::PeerRenamed {
                    chat,
                    peer,
                    name: n.clone(),
                });
                // Past messages keep the name they were sent under; a change is announced.
                if !old.is_empty() {
                    self.emit(Event::Notice {
                        chat,
                        notice: Notice::Renamed { old, name: n },
                    });
                }
            }
            Ctrl::Verified => {
                let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
                    return;
                };
                p.verified_peer = true;
                let by_me = p.verified_me;
                self.emit(Event::PeerVerified {
                    chat,
                    peer,
                    by_me,
                    by_peer: true,
                });
                self.on_verified(chat, tag);
            }
            Ctrl::Lock(locked) => {
                let Some(c) = self.chats.get_mut(&chat) else { return };
                if !c.group || c.locked == locked {
                    return;
                }
                c.locked = locked;
                let by = c.peers.get(tag).map(|p| p.name.clone()).unwrap_or_default();
                self.emit(Event::Locked { chat, locked });
                self.emit(Event::Notice {
                    chat,
                    notice: Notice::Locked { by, locked },
                });
                self.refresh_discovery();
            }
            Ctrl::Card(card) => self.on_card(chat, tag, &card),
        }
    }

    fn on_chat(&mut self, chat: ChatId, tag: &PeerTag, m: ChatMsg) {
        let now = now_ms();
        let peer = tag_hex(tag);
        let Some(p) = self.chats.get(&chat).and_then(|c| c.peers.get(tag)) else {
            return;
        };
        let author = p.name.clone();
        let verified = p.verified();
        if let ChatMsg::Msg { id, .. } | ChatMsg::File(FileMeta { id, .. }) = &m {
            let Some(c) = self.chats.get_mut(&chat) else { return };
            if c.authors.contains_key(id) {
                return;
            }
            c.authors.insert(id.clone(), Some(*tag));
        }
        match m {
            ChatMsg::Msg { id, text, ttl } => {
                let text = clean_text(&text);
                let expires_at = ttl.map(|t| now + t * 1000);
                self.keep_history(chat, &id, false, now, &text, expires_at);
                self.emit(Event::Message {
                    chat,
                    peer: Some(peer),
                    id,
                    author,
                    text,
                    ts: now,
                    expires_at,
                    past: false,
                });
            }
            ChatMsg::Del { id } => {
                let Some(c) = self.chats.get_mut(&chat) else { return };
                if c.authors.get(&id) != Some(&Some(*tag)) {
                    return;
                }
                c.authors.remove(&id);
                self.forget_history(chat, &id);
                self.discard_received(chat, &id);
                self.emit(Event::MessageDeleted { chat, id });
            }
            ChatMsg::File(meta) => {
                // The name's extension must agree too: it decides how the file opens once saved.
                let auto = proto::previewable(&meta.mime)
                    && proto::previewable(guess_mime(&meta.name))
                    && meta.size <= AUTO_DOWNLOAD
                    && verified;
                let id = meta.id.clone();
                if let Some(t) = meta.ttl
                    && let Some(c) = self.chats.get_mut(&chat)
                {
                    c.expiry.insert(id.clone(), now + t * 1000);
                }
                self.emit(Event::FileOffered {
                    chat,
                    peer: Some(peer),
                    id: meta.id.clone(),
                    name: meta.name.clone(),
                    size: meta.size,
                    mime: meta.mime.clone(),
                    expires_at: meta.ttl.map(|t| now + t * 1000),
                });
                let busy: usize = self
                    .chats
                    .get(&chat)
                    .map(|c| c.peers.values().map(|p| p.downloads.len()).sum())
                    .unwrap_or(0);
                if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) {
                    p.offers.insert(id.clone(), meta);
                }
                if auto && busy < AUTO_DOWNLOADS {
                    self.request_file(chat, &id);
                }
            }
            ChatMsg::FileGet { id } => self.start_upload(chat, tag, &id),
            ChatMsg::FileEnd { id } => {
                if self.chats.get(&chat).is_some_and(|c| c.outgoing.contains_key(&id)) {
                    self.emit(Event::FileDelivered { chat, id, peer });
                }
            }
            ChatMsg::FileCancel { id } => {
                let removed = self
                    .chats
                    .get_mut(&chat)
                    .and_then(|c| c.peers.get_mut(tag))
                    .and_then(|p| p.downloads.remove(&id));
                if let Some(dl) = removed {
                    if let Some((_, path)) = dl.file {
                        let _ = std::fs::remove_file(path);
                    }
                    self.emit(Event::FileFailed {
                        chat,
                        id,
                        reason: "cancelled".into(),
                    });
                }
            }
        }
    }

    fn send_text(&mut self, chat: ChatId, text: &str) {
        let text = text.trim();
        if text.is_empty() || text.chars().count() > proto::MAX_TEXT {
            return;
        }
        let Some(c) = self.chats.get(&chat) else { return };
        if c.ended {
            return;
        }
        let id = new_id();
        let ts = now_ms();
        let mut v = json!({"t": "msg", "id": id, "ts": ts, "text": text});
        let expires_at = (self.cfg.ttl > 0).then(|| {
            v["ttl"] = json!(self.cfg.ttl);
            ts + self.cfg.ttl * 1000
        });
        self.broadcast_app(chat, lane::CHAT, &v);
        if let Some(c) = self.chats.get_mut(&chat) {
            c.authors.insert(id.clone(), None);
        }
        self.keep_history(chat, &id, true, ts, text, expires_at);
        self.emit(Event::Message {
            chat,
            peer: None,
            id,
            author: self.cfg.name.clone(),
            text: text.to_string(),
            ts,
            expires_at,
            past: false,
        });
    }

    fn delete_message(&mut self, chat: ChatId, id: &str) {
        self.broadcast_app(chat, lane::CHAT, &json!({"t": "del", "id": id}));
        if let Some(c) = self.chats.get_mut(&chat) {
            c.authors.remove(id);
        }
        self.forget_history(chat, id);
        self.discard_received(chat, id);
        self.emit(Event::MessageDeleted {
            chat,
            id: id.to_string(),
        });
    }

    fn verify(&mut self, chat: ChatId, peer: Option<&str>, ok: bool) {
        let tags: Vec<PeerTag> = match peer.and_then(parse_tag) {
            Some(t) => vec![t],
            None => self
                .chats
                .get(&chat)
                .map(|c| c.peers.keys().copied().collect())
                .unwrap_or_default(),
        };
        for tag in tags {
            if !ok {
                // The emojis differ: someone is in the middle. End with this peer at once.
                if let Some(bye) = self
                    .chats
                    .get_mut(&chat)
                    .and_then(|c| c.peers.get_mut(&tag))
                    .and_then(|p| p.session.close())
                {
                    self.send_frame(chat, &tag, lane::CTRL, bye);
                }
                let group = self.chats.get(&chat).is_some_and(|c| c.group);
                if group {
                    self.drop_peer(chat, &tag, false);
                } else {
                    self.end_chat(chat, EndReason::Mismatch);
                }
                continue;
            }
            let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(&tag)) else {
                continue;
            };
            p.verified_me = true;
            let by_peer = p.verified_peer;
            self.send_app(chat, &tag, lane::CTRL, json!({"t": "verified"}));
            self.emit(Event::PeerVerified {
                chat,
                peer: tag_hex(&tag),
                by_me: true,
                by_peer,
            });
            self.on_verified(chat, &tag);
        }
    }

    fn set_locked(&mut self, chat: ChatId, locked: bool) {
        let Some(c) = self.chats.get_mut(&chat) else { return };
        if !c.group || c.locked == locked {
            return;
        }
        c.locked = locked;
        self.broadcast_app(chat, lane::CTRL, &json!({"t": "lock", "locked": locked}));
        self.emit(Event::Locked { chat, locked });
        self.refresh_discovery();
    }

    /// Kept history of a saved contact who has it turned on; never disappearing messages.
    fn keep_history(&self, chat: ChatId, id: &str, mine: bool, ts: u64, text: &str, expires_at: Option<u64>) {
        if expires_at.is_some() {
            return;
        }
        let Some(key) = self.chats.get(&chat).and_then(|c| c.contact_key()) else {
            return;
        };
        if self.store.contact(key).is_some_and(|c| c.history) {
            let _ = self.store.append_history(
                key,
                HistoryEntry {
                    id: id.to_string(),
                    mine,
                    ts,
                    text: text.to_string(),
                    expires_at: None,
                },
            );
        }
    }

    /// Deletes a file we received, where the app keeps them (`Config::discard_files`).
    fn discard_received(&mut self, chat: ChatId, id: &str) {
        if !self.cfg.discard_files {
            return;
        }
        if let Some(path) = self.chats.get_mut(&chat).and_then(|c| c.received.remove(id)) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn forget_history(&self, chat: ChatId, id: &str) {
        if let Some(key) = self.chats.get(&chat).and_then(|c| c.contact_key())
            && self.store.is_unlocked()
        {
            let _ = self.store.remove_history(key, id);
        }
    }

    // ── files ───────────────────────────────────────────────────────────────

    fn offer_file(&mut self, chat: ChatId, path: PathBuf) -> Result<()> {
        let Some(c) = self.chats.get(&chat) else {
            bail!("no such conversation")
        };
        if c.ended {
            bail!("the conversation ended");
        }
        let mut f = std::fs::File::open(&path)?;
        let size = f.metadata()?.len();
        if size > proto::MAX_FILE {
            bail!("files are limited to 4 GB");
        }
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
        let ts = now_ms();
        let mut offer = json!({"t": "file", "id": id, "ts": ts, "name": name, "size": size, "mime": mime, "sha256": hex::encode(h.finalize())});
        let expires_at = (self.cfg.ttl > 0).then(|| {
            offer["ttl"] = json!(self.cfg.ttl);
            ts + self.cfg.ttl * 1000
        });
        if let Some(c) = self.chats.get_mut(&chat) {
            c.outgoing.insert(id.clone(), OutFile { path, size });
            c.authors.insert(id.clone(), None);
        }
        self.broadcast_app(chat, lane::CHAT, &offer);
        self.emit(Event::FileOffered {
            chat,
            peer: None,
            id,
            name,
            size,
            mime: mime.to_string(),
            expires_at,
        });
        Ok(())
    }

    fn start_upload(&mut self, chat: ChatId, tag: &PeerTag, id: &str) {
        let Some(c) = self.chats.get(&chat) else { return };
        let Some(out) = c.outgoing.get(id) else { return };
        let Some(p) = c.peers.get(tag) else { return };
        // One upload of a file per peer, a few at once, and only on a path that carries files:
        // asking again and again must not open the file again and again.
        let carries = p.rtc_open || p.local.is_some_and(|l| self.links.contains_key(&l));
        if !carries || p.uploads.len() >= MAX_UPLOADS || p.uploads.iter().any(|u| u.id == id) {
            return;
        }
        let Ok(file) = std::fs::File::open(&out.path) else {
            return;
        };
        let size = out.size;
        if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) {
            p.uploads.push(Upload {
                id: id.to_string(),
                file,
                index: 0,
                sent: 0,
                size,
            });
        }
        self.pump(chat, tag);
    }

    /// Sends file chunks while the path has room.
    fn pump(&mut self, chat: ChatId, tag: &PeerTag) {
        let Some(p) = self.chats.get(&chat).and_then(|c| c.peers.get(tag)) else {
            return;
        };
        let local = p.local.filter(|l| self.links.contains_key(l));
        if local.is_none() && !p.rtc_open {
            return;
        }
        loop {
            let room = match local {
                Some(l) => self.links.get(&l).is_some_and(LocalLink::has_room),
                None => self
                    .chats
                    .get(&chat)
                    .and_then(|c| c.peers.get(tag))
                    .is_some_and(|p| p.in_flight < IN_FLIGHT),
            };
            if !room {
                break;
            }
            let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
                return;
            };
            let Some(up) = p.uploads.first_mut() else { break };
            if up.sent >= up.size {
                p.uploads.remove(0);
                continue;
            }
            let mut buf = vec![0u8; proto::CHUNK_SIZE.min((up.size - up.sent) as usize)];
            if up.file.read_exact(&mut buf).is_err() {
                p.uploads.remove(0);
                continue;
            }
            let c = proto::chunk(&up.id, up.index, &buf);
            up.index += 1;
            up.sent += buf.len() as u64;
            let Ok(frame) = p.session.seal(lane::FILE, &c) else {
                break;
            };
            match local {
                Some(l) => self.local_write(l, lane::FILE, &frame),
                None => {
                    p.in_flight += 1;
                    if let Some(r) = &p.rtc {
                        let _ = r.send(RtcCmd::SendTracked(usize::from(lane::FILE), frame));
                    }
                }
            }
        }
    }

    fn fail_wanted(&mut self, chat: ChatId, tag: &PeerTag) {
        let wanted = self
            .chats
            .get_mut(&chat)
            .and_then(|c| c.peers.get_mut(tag))
            .map(|p| std::mem::take(&mut p.wanted))
            .unwrap_or_default();
        for id in wanted {
            self.emit(Event::FileFailed {
                chat,
                id,
                reason: "files need a direct connection".into(),
            });
        }
    }

    fn request_file(&mut self, chat: ChatId, id: &str) {
        let tags: Vec<PeerTag> = self
            .chats
            .get(&chat)
            .map(|c| c.peers.keys().copied().collect())
            .unwrap_or_default();
        for tag in tags {
            let local_ok = self
                .chats
                .get(&chat)
                .and_then(|c| c.peers.get(&tag))
                .and_then(|p| p.local)
                .is_some_and(|l| self.links.contains_key(&l));
            let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(&tag)) else {
                continue;
            };
            if !p.offers.contains_key(id) {
                continue;
            }
            if !local_ok && !p.rtc_open {
                if p.relayed {
                    self.emit(Event::Notice {
                        chat,
                        notice: Notice::FilesNeedDirect,
                    });
                    self.emit(Event::FileFailed {
                        chat,
                        id: id.to_string(),
                        reason: "files need a direct connection".into(),
                    });
                } else if !p.wanted.iter().any(|w| w == id) {
                    p.wanted.push(id.to_string());
                }
                return;
            }
            let Some(meta) = p.offers.remove(id) else { continue };
            let size = meta.size;
            p.downloads.insert(
                id.to_string(),
                Download {
                    meta,
                    got: 0,
                    next: 0,
                    hasher: Sha256::new(),
                    file: None,
                },
            );
            if size == 0 {
                self.finish_download(chat, &tag, id);
            } else {
                self.send_app(chat, &tag, lane::CHAT, json!({"t": "file-get", "id": id}));
            }
            return;
        }
    }

    fn cancel_file(&mut self, chat: ChatId, id: &str) {
        let tags: Vec<PeerTag> = self
            .chats
            .get(&chat)
            .map(|c| c.peers.keys().copied().collect())
            .unwrap_or_default();
        for tag in tags {
            let dl = self
                .chats
                .get_mut(&chat)
                .and_then(|c| c.peers.get_mut(&tag))
                .and_then(|p| p.downloads.remove(id));
            if let Some(dl) = dl {
                if let Some((_, path)) = dl.file {
                    let _ = std::fs::remove_file(path);
                }
                self.send_app(chat, &tag, lane::CHAT, json!({"t": "file-cancel", "id": id}));
            }
        }
    }

    fn on_chunk(&mut self, chat: ChatId, tag: &PeerTag, data: &[u8]) {
        let Some((id, index, body)) = proto::parse_chunk(data) else {
            return;
        };
        let out_dir = self.cfg.out_dir.clone();
        let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
            return;
        };
        let Some(dl) = p.downloads.get_mut(&id) else { return };
        if index != dl.next || dl.got + body.len() as u64 > dl.meta.size {
            if let Some(dl) = p.downloads.remove(&id)
                && let Some((_, path)) = dl.file
            {
                let _ = std::fs::remove_file(path);
            }
            self.emit(Event::FileFailed {
                chat,
                id,
                reason: "chunks out of order".into(),
            });
            return;
        }
        if dl.file.is_none() {
            match create_unique(&out_dir, &sanitize_file_name(&dl.meta.name)) {
                Ok(x) => dl.file = Some(x),
                Err(e) => {
                    p.downloads.remove(&id);
                    self.emit(Event::FileFailed {
                        chat,
                        id,
                        reason: e.to_string(),
                    });
                    return;
                }
            }
        }
        if let Some((f, _)) = dl.file.as_mut()
            && let Err(e) = f.write_all(body)
        {
            // A full disk: the file would be saved truncated.
            if let Some(dl) = p.downloads.remove(&id)
                && let Some((_, path)) = dl.file
            {
                let _ = std::fs::remove_file(path);
            }
            self.emit(Event::FileFailed {
                chat,
                id,
                reason: e.to_string(),
            });
            return;
        }
        dl.hasher.update(body);
        dl.got += body.len() as u64;
        dl.next += 1;
        let (done, total) = (dl.got, dl.meta.size);
        self.emit(Event::FileProgress {
            chat,
            id: id.clone(),
            done,
            total,
        });
        if done == total {
            self.finish_download(chat, tag, &id);
        }
    }

    fn finish_download(&mut self, chat: ChatId, tag: &PeerTag, id: &str) {
        let out_dir = self.cfg.out_dir.clone();
        let Some(dl) = self
            .chats
            .get_mut(&chat)
            .and_then(|c| c.peers.get_mut(tag))
            .and_then(|p| p.downloads.remove(id))
        else {
            return;
        };
        let (file, path) = match dl.file {
            Some(x) => x,
            None => match create_unique(&out_dir, &sanitize_file_name(&dl.meta.name)) {
                Ok(x) => x,
                Err(e) => {
                    self.emit(Event::FileFailed {
                        chat,
                        id: id.to_string(),
                        reason: e.to_string(),
                    });
                    return;
                }
            },
        };
        drop(file);
        if hex::encode(dl.hasher.finalize()) == dl.meta.sha256 {
            if let Some(c) = self.chats.get_mut(&chat) {
                c.received.insert(id.to_string(), path.clone());
            }
            self.send_app(chat, tag, lane::CHAT, json!({"t": "file-end", "id": id}));
            self.emit(Event::FileSaved {
                chat,
                id: id.to_string(),
                path,
            });
        } else {
            let _ = std::fs::remove_file(&path);
            self.emit(Event::FileFailed {
                chat,
                id: id.to_string(),
                reason: "SHA-256 mismatch: file discarded".into(),
            });
        }
    }

    // ── nearby links ────────────────────────────────────────────────────────

    fn on_local(&mut self, ev: LocalIn) {
        match ev {
            LocalIn::Opened {
                link,
                kind,
                medium,
                token,
            } => {
                if !self.cfg.local || self.links.len() >= local::MAX_LINKS {
                    if let Some(h) = &self.host {
                        h.close(link);
                    }
                    return;
                }
                let channel = token.and_then(|t| {
                    self.rdv
                        .tokens(now_ms(), true)
                        .into_iter()
                        .find(|(_, tok)| tok.as_slice() == t.as_slice())
                        .map(|(ch, _)| ch)
                });
                self.links.insert(link, LocalLink::new(kind, medium, channel));
                self.rdv.local_opened(link, channel);
                if let Some(ch) = channel {
                    self.on_dialed(link, ch);
                }
            }
            LocalIn::Data { link, data } => self.on_local_data(link, &data),
            LocalIn::Sent { link, bytes } => {
                let peer = self.links.get_mut(&link).and_then(|l| {
                    l.sent(bytes);
                    l.peer
                });
                if let Some((chat, tag)) = peer {
                    self.pump(chat, &tag);
                }
            }
            LocalIn::Closed { link } => self.forget_link(link),
        }
    }

    /// We dialed a device for `ch`: say on it what we would say on the network.
    fn on_dialed(&mut self, _link: LinkId, ch: ChannelId) {
        if let Some(&chat) = self.link_channels.get(&ch) {
            let hs1 = self
                .chats
                .get(&chat)
                .and_then(|c| c.link.as_ref())
                .and_then(|l| l.hs1.clone());
            let waiting = self.chats.get(&chat).is_some_and(|c| c.peers.is_empty());
            if let Some(hs1) = hs1
                && waiting
            {
                self.send_hs1(chat, &hs1);
            }
            return;
        }
        self.contact_dialed(ch);
    }

    fn on_local_data(&mut self, link: LinkId, data: &[u8]) {
        let Some(l) = self.links.get_mut(&link) else { return };
        let Ok(records) = l.push(data) else {
            self.close_link(link);
            return;
        };
        for (lane, payload) in records {
            if lane == hush_core::stream::SIGNAL_LANE {
                match self.rdv.open_local(link, &payload) {
                    Some((ch, env)) => {
                        if let Some(l) = self.links.get_mut(&link) {
                            l.useful = true;
                        }
                        self.on_envelope(ch, env, Via::Local(link));
                    }
                    None => self.strike(link),
                }
                continue;
            }
            let peer = self.links.get(&link).and_then(|l| l.peer);
            match peer {
                Some((chat, tag)) => self.on_frame(chat, &tag, &payload, Some(link)),
                None => self.strike(link),
            }
        }
    }

    fn strike(&mut self, link: LinkId) {
        if self.links.get_mut(&link).is_some_and(|l| l.strike().is_err()) {
            self.close_link(link);
        }
    }

    fn close_link(&mut self, link: LinkId) {
        if let Some(h) = &self.host {
            h.close(link);
        }
        self.forget_link(link);
    }

    fn forget_link(&mut self, link: LinkId) {
        let Some(l) = self.links.remove(&link) else { return };
        self.rdv.local_closed(link);
        if let Some((chat, tag)) = l.peer {
            if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(&tag)) {
                p.local = None;
                // Without a link, a direct path may still come: give it its full chance.
                p.since = Instant::now();
            }
            self.refresh_path(chat, &tag);
        }
    }

    fn local_write(&mut self, link: LinkId, lane: u8, data: &[u8]) {
        let Some(l) = self.links.get_mut(&link) else { return };
        let writes = l.encode(lane, data);
        if let Some(h) = &self.host {
            for w in writes {
                h.send(link, w);
            }
        }
    }

    fn local_write_envelope(&mut self, link: LinkId, payload: &[u8]) {
        let Some(l) = self.links.get_mut(&link) else { return };
        let writes = l.envelope(payload);
        if let Some(h) = &self.host {
            for w in writes {
                h.send(link, w);
            }
        }
    }

    /// Tells the app which tokens to advertise and to look for:
    /// - a link we created (or a group we are in) is advertised while it accepts people;
    /// - a link we opened is looked for until we are connected;
    /// - saved contacts not connected: the side that starts handshakes looks, the other
    ///   advertises (so the two do not dial each other at once).
    pub(crate) fn refresh_discovery(&mut self) {
        let Some(host) = self.host.clone() else { return };
        let (mut adv, mut scan) = (Vec::new(), Vec::new());
        if self.cfg.local {
            let now = now_ms();
            let current = self.rdv.tokens(now, false);
            let window = self.rdv.tokens(now, true);
            let tokens = |ch: ChannelId, list: &[(ChannelId, [u8; 16])]| -> Vec<Vec<u8>> {
                list.iter().filter(|(c, _)| *c == ch).map(|(_, t)| t.to_vec()).collect()
            };
            for (ch, chat) in &self.link_channels {
                let Some(c) = self.chats.get(chat) else { continue };
                if c.ended {
                    continue;
                }
                let creator = c.link.as_ref().is_some_and(|l| l.creator);
                let open = if c.group {
                    !c.locked && c.peers.len() < MAX_GROUP - 1
                } else {
                    c.peers.is_empty()
                };
                if (creator || (c.group && !c.peers.is_empty())) && open {
                    adv.extend(tokens(*ch, &current));
                }
                if !creator && c.peers.is_empty() {
                    scan.extend(tokens(*ch, &window));
                }
            }
            for (ch, dial) in self.contact_discovery() {
                if dial {
                    scan.extend(tokens(ch, &window));
                } else {
                    adv.extend(tokens(ch, &current));
                }
            }
        }
        adv.sort();
        adv.dedup();
        scan.sort();
        scan.dedup();
        if adv != self.advertised {
            self.advertised = adv.clone();
            host.advertise(adv);
        }
        if scan != self.scanned {
            self.scanned = scan.clone();
            host.scan(scan);
        }
    }
}

/// Files a peer was still sending: half a file is of no use to anyone.
fn discard_partial(p: &Peer) {
    for dl in p.downloads.values() {
        if let Some((_, path)) = &dl.file {
            let _ = std::fs::remove_file(path);
        }
    }
}
