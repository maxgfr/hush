//! Signaling transports (Nostr relays, WebTorrent trackers, and Bluetooth or Wi-Fi links to nearby
//! devices) and the rendezvous layer, as in `web/src/signal/{nostr,torrent}.ts` and
//! `web/src/net/rendezvous.ts`. Transports only ever carry envelopes sealed by the core, under
//! daily rotating topics.

use crate::util::{b64u, from_b64u, now_ms, random};
use futures_util::{SinkExt, StreamExt};
use hush_core::envelope::{self, Envelope, PeerTag, ReplayGuard};
use k256::schnorr::SigningKey;
use serde_json::{Value, json};
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// The largest message a relay or tracker may send: one envelope, which carries at most one
/// session record, in base64 inside JSON. A server cannot make us take in more at once.
const MAX_SERVER_MESSAGE: usize = 4 * hush_core::stream::MAX_RECORD / 3 + 64 * 1024;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(url: &str) -> Option<Socket> {
    let config = WebSocketConfig {
        max_message_size: Some(MAX_SERVER_MESSAGE),
        max_frame_size: Some(MAX_SERVER_MESSAGE),
        ..Default::default()
    };
    let (ws, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), false)
        .await
        .ok()?;
    Some(ws)
}

pub const NOSTR_KIND: u64 = 21_918;
pub const DEFAULT_RELAYS: [&str; 4] = [
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
    "wss://nostr.mom",
];
pub const DEFAULT_TRACKERS: [&str; 2] = ["wss://tracker.openwebtorrent.com", "wss://tracker.webtorrent.dev"];

#[derive(Clone, Debug)]
pub enum OutCmd {
    Join([u8; 32]),
    Leave([u8; 32]),
    Send { topic: [u8; 32], wire: String },
}

pub type Inbound = mpsc::UnboundedSender<(String, [u8; 32])>;

fn backoff(attempt: u32) -> Duration {
    let base = 1000u64.saturating_mul(1 << attempt.min(5)).min(30_000);
    Duration::from_millis(base / 2 + rand::random::<u64>() % (base / 2 + 1))
}

// ── Nostr ───────────────────────────────────────────────────────────────────

fn sub_id(hex_topic: &str) -> String {
    format!("hush-{}", &hex_topic[..16])
}

fn nostr_req(hex_topic: &str) -> String {
    let since = now_ms() / 1000 - 60;
    json!(["REQ", sub_id(hex_topic), {"kinds": [NOSTR_KIND], "#t": [hex_topic], "since": since}]).to_string()
}

fn sign_event(sk: &SigningKey, content: &str, tags: Value) -> Value {
    let pubkey = hex::encode(sk.verifying_key().to_bytes());
    let created_at = now_ms() / 1000;
    let serial = json!([0, pubkey, created_at, NOSTR_KIND, tags, content]).to_string();
    let id = hush_core::kdf::sha256(&[serial.as_bytes()]);
    let sig = sk.sign_raw(&id, &random::<32>()).expect("schnorr signing");
    json!({"id": hex::encode(id), "pubkey": pubkey, "created_at": created_at, "kind": NOSTR_KIND, "tags": tags, "content": content, "sig": hex::encode(sig.to_bytes())})
}

fn track(cmd: &OutCmd, topics: &mut HashMap<String, [u8; 32]>, key: impl Fn(&[u8; 32]) -> String) {
    match cmd {
        OutCmd::Join(t) => {
            topics.insert(key(t), *t);
        }
        OutCmd::Leave(t) => {
            topics.remove(&key(t));
        }
        OutCmd::Send { .. } => {}
    }
}

/// Waits out a reconnect delay while still tracking topic changes. False when the sender is gone.
async fn wait_offline(
    rx: &mut mpsc::UnboundedReceiver<OutCmd>,
    topics: &mut HashMap<String, [u8; 32]>,
    delay: Duration,
    key: impl Fn(&[u8; 32]) -> String + Copy,
) -> bool {
    let wait = tokio::time::sleep(delay);
    tokio::pin!(wait);
    loop {
        tokio::select! {
            _ = &mut wait => return true,
            cmd = rx.recv() => match cmd { Some(c) => track(&c, topics, key), None => return false },
        }
    }
}

pub fn spawn_nostr(url: String, inbound: Inbound) -> mpsc::UnboundedSender<OutCmd> {
    let sk = SigningKey::random(&mut rand::rngs::OsRng);
    let own = hex::encode(sk.verifying_key().to_bytes());
    let (tx, mut rx) = mpsc::unbounded_channel::<OutCmd>();
    let key = |t: &[u8; 32]| hex::encode(t);
    tokio::spawn(async move {
        let mut topics: HashMap<String, [u8; 32]> = HashMap::new();
        let mut attempt = 0u32;
        loop {
            let conn = tokio::time::timeout(Duration::from_secs(10), connect(&url)).await;
            let Ok(Some(mut ws)) = conn else {
                if !wait_offline(&mut rx, &mut topics, backoff(attempt), key).await {
                    return;
                }
                attempt += 1;
                continue;
            };
            attempt = 0;
            for hex_topic in topics.keys() {
                let _ = ws.send(Message::text(nostr_req(hex_topic))).await;
            }
            loop {
                tokio::select! {
                    msg = ws.next() => match msg {
                        Some(Ok(Message::Text(t))) => {
                            let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                            if v.get(0).and_then(Value::as_str) != Some("EVENT") { continue; }
                            let ev = &v[2];
                            if ev["pubkey"].as_str() == Some(own.as_str()) || ev["kind"].as_u64() != Some(NOSTR_KIND) { continue; }
                            let Some(content) = ev["content"].as_str() else { continue };
                            let tag = ev["tags"].as_array().and_then(|tags| tags.iter().find(|t| t[0] == "t")).and_then(|t| t[1].as_str());
                            if let Some(topic) = tag.and_then(|h| topics.get(h)) { let _ = inbound.send((content.to_string(), *topic)); }
                        }
                        Some(Ok(Message::Ping(p))) => { let _ = ws.send(Message::Pong(p)).await; }
                        Some(Ok(_)) => {}
                        _ => break,
                    },
                    cmd = rx.recv() => {
                        let Some(cmd) = cmd else { let _ = ws.close(None).await; return };
                        let frame = match &cmd {
                            OutCmd::Join(t) => { let h = key(t); let f = (!topics.contains_key(&h)).then(|| nostr_req(&h)); topics.insert(h, *t); f }
                            OutCmd::Leave(t) => { let h = key(t); topics.remove(&h).map(|_| json!(["CLOSE", sub_id(&h)]).to_string()) }
                            OutCmd::Send { topic, wire } => Some(json!(["EVENT", sign_event(&sk, wire, json!([["t", key(topic)]]))]).to_string()),
                        };
                        if let Some(f) = frame && ws.send(Message::text(f)).await.is_err() { break; }
                    }
                }
            }
            tokio::time::sleep(backoff(attempt)).await;
            attempt += 1;
        }
    });
    tx
}

// ── WebTorrent tracker ──────────────────────────────────────────────────────

/// The tracker protocol carries binary ids as latin1 strings.
pub fn info_hash(topic: &[u8; 32]) -> String {
    topic[..20].iter().map(|&b| b as char).collect()
}

fn announce(hash: &str, peer_id: &str, event: Option<&str>) -> String {
    let mut v = json!({"action": "announce", "info_hash": hash, "peer_id": peer_id, "numwant": 0, "offers": []});
    if let Some(e) = event {
        v["event"] = json!(e);
    }
    v.to_string()
}

pub fn spawn_tracker(url: String, peer_id: String, inbound: Inbound) -> mpsc::UnboundedSender<OutCmd> {
    let (tx, mut rx) = mpsc::unbounded_channel::<OutCmd>();
    tokio::spawn(async move {
        let mut topics: HashMap<String, [u8; 32]> = HashMap::new();
        let mut attempt = 0u32;
        loop {
            let conn = tokio::time::timeout(Duration::from_secs(10), connect(&url)).await;
            let Ok(Some(mut ws)) = conn else {
                if !wait_offline(&mut rx, &mut topics, backoff(attempt), info_hash).await {
                    return;
                }
                attempt += 1;
                continue;
            };
            attempt = 0;
            for h in topics.keys() {
                let _ = ws.send(Message::text(announce(h, &peer_id, Some("started")))).await;
            }
            let mut reannounce = tokio::time::interval(Duration::from_secs(50));
            reannounce.tick().await;
            loop {
                tokio::select! {
                    _ = reannounce.tick() => {
                        for h in topics.keys() { let _ = ws.send(Message::text(announce(h, &peer_id, None))).await; }
                    }
                    msg = ws.next() => match msg {
                        Some(Ok(Message::Text(t))) => {
                            let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                            if v["action"] != "announce" || v["peer_id"].as_str() == Some(peer_id.as_str()) { continue; }
                            let Some(topic) = v["info_hash"].as_str().and_then(|h| topics.get(h)) else { continue };
                            let sdp = v["offer"]["sdp"].as_str().or_else(|| v["answer"]["sdp"].as_str());
                            if let Some(wire) = sdp { let _ = inbound.send((wire.to_string(), *topic)); }
                        }
                        Some(Ok(Message::Ping(p))) => { let _ = ws.send(Message::Pong(p)).await; }
                        Some(Ok(_)) => {}
                        _ => break,
                    },
                    cmd = rx.recv() => {
                        let Some(cmd) = cmd else { let _ = ws.close(None).await; return };
                        let frame = match &cmd {
                            OutCmd::Join(t) => { let h = info_hash(t); let f = announce(&h, &peer_id, Some("started")); topics.insert(h, *t); Some(f) }
                            OutCmd::Leave(t) => { let h = info_hash(t); topics.remove(&h).map(|_| announce(&h, &peer_id, Some("stopped"))) }
                            OutCmd::Send { topic, wire } => {
                                let offers: Vec<Value> = (0..10).map(|_| json!({"offer_id": b64u(&random::<15>()), "offer": {"type": "offer", "sdp": wire}})).collect();
                                Some(json!({"action": "announce", "info_hash": info_hash(topic), "peer_id": peer_id, "numwant": 10, "offers": offers}).to_string())
                            }
                        };
                        if let Some(f) = frame && ws.send(Message::text(f)).await.is_err() { break; }
                    }
                }
            }
            tokio::time::sleep(backoff(attempt)).await;
            attempt += 1;
        }
    });
    tx
}

// ── rendezvous ──────────────────────────────────────────────────────────────

/// Where envelopes of one secret (a link, a session, a contact) are sealed and opened.
pub struct Channel {
    key: [u8; 32],
    topic: Box<dyn Fn(u32) -> [u8; 32] + Send>,
    guard: ReplayGuard,
}

/// How long a Bluetooth or Wi-Fi discovery token lasts.
pub const TOKEN_SLOT_MS: u64 = 10 * 60_000;

impl Channel {
    pub fn new(key: [u8; 32], topic: impl Fn(u32) -> [u8; 32] + Send + 'static) -> Self {
        Self {
            key,
            topic: Box::new(topic),
            guard: ReplayGuard::default(),
        }
    }

    /// A channel anyone can seal on (its key derives from public data, like an identifier's):
    /// a flood of envelopes cannot shut it, at the cost of replays once it is full.
    pub fn public(key: [u8; 32], topic: impl Fn(u32) -> [u8; 32] + Send + 'static) -> Self {
        Self {
            key,
            topic: Box::new(topic),
            guard: ReplayGuard::lossy(16_384),
        }
    }

    /// The token nearby devices advertise for this channel during a 10-minute slot. It derives
    /// from the envelope key, not the topic, so a relay that sees the topic cannot recognize it.
    pub fn token(&self, slot: u64) -> [u8; 16] {
        let mac = hush_core::kdf::hmac(&self.key, &[b"hush/v2/local/adv", &slot.to_be_bytes()]);
        mac[..16].try_into().expect("16 bytes")
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.key = [0u8; 32];
    }
}

pub type ChannelId = u32;
/// A Bluetooth or Wi-Fi link to a nearby device, numbered by the app.
pub type LinkId = u64;

#[derive(Clone, Debug, Default)]
pub struct SignalConfig {
    pub relays: Vec<String>,
    pub trackers: Vec<String>,
}

/// How an envelope reached us.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Network,
    Manual,
    Local(LinkId),
}

impl Via {
    /// The nearby link it came over.
    pub fn link(&self) -> Option<LinkId> {
        match self {
            Via::Local(l) => Some(*l),
            _ => None,
        }
    }
}

pub struct Rendezvous {
    pub tag: PeerTag,
    outs: Vec<mpsc::UnboundedSender<OutCmd>>,
    rx: mpsc::UnboundedReceiver<(String, [u8; 32])>,
    channels: HashMap<ChannelId, (Channel, Vec<[u8; 32]>)>,
    topics: HashMap<[u8; 32], ChannelId>,
    next_id: ChannelId,
    day: u32,
    seen: HashSet<Vec<u8>>,
    /// Manual mode: envelopes are printed as codes instead of (or as well as) sent.
    pub manual: bool,
    /// The channels each nearby link has carried: envelopes of a channel go to those links.
    local: HashMap<LinkId, HashSet<ChannelId>>,
    /// Envelopes for nearby links: `(link, topic || envelope)`, sent by the engine.
    local_out: mpsc::UnboundedSender<(LinkId, Vec<u8>)>,
    /// Which nearby link each peer (by channel and tag) was last heard on.
    local_peers: HashMap<(ChannelId, PeerTag), LinkId>,
    inbound: Inbound,
    peer_id: String,
}

impl Rendezvous {
    /// `local_out` receives what must go to nearby links (`topic || envelope`).
    pub fn new(cfg: &SignalConfig, local_out: mpsc::UnboundedSender<(LinkId, Vec<u8>)>) -> Self {
        let tag: PeerTag = random();
        let peer_id: String = b64u(&tag).chars().take(20).collect();
        let (in_tx, rx) = mpsc::unbounded_channel();
        let mut outs = Vec::new();
        for r in &cfg.relays {
            outs.push(spawn_nostr(r.clone(), in_tx.clone()));
        }
        for t in &cfg.trackers {
            outs.push(spawn_tracker(t.clone(), peer_id.clone(), in_tx.clone()));
        }
        Self {
            tag,
            outs,
            rx,
            channels: HashMap::new(),
            topics: HashMap::new(),
            next_id: 1,
            day: hush_core::utc_day(now_ms()),
            seen: HashSet::new(),
            manual: false,
            local: HashMap::new(),
            local_out,
            local_peers: HashMap::new(),
            inbound: in_tx,
            peer_id,
        }
    }

    /// Replaces the relays and trackers, keeping every channel joined.
    pub fn reconfigure(&mut self, cfg: &SignalConfig) {
        self.outs.clear(); // dropping the senders ends their tasks
        for r in &cfg.relays {
            self.outs.push(spawn_nostr(r.clone(), self.inbound.clone()));
        }
        for t in &cfg.trackers {
            self.outs
                .push(spawn_tracker(t.clone(), self.peer_id.clone(), self.inbound.clone()));
        }
        let topics: Vec<[u8; 32]> = self.topics.keys().copied().collect();
        for t in topics {
            self.broadcast(OutCmd::Join(t));
        }
    }

    /// The nearby link a peer of this channel was heard on, if it is still open.
    pub fn local_peer(&self, ch: ChannelId, tag: &PeerTag) -> Option<LinkId> {
        let link = *self.local_peers.get(&(ch, *tag))?;
        self.local.contains_key(&link).then_some(link)
    }

    /// A nearby link opened. `channel` is the one it was found for, if we dialed it.
    pub fn local_opened(&mut self, link: LinkId, channel: Option<ChannelId>) {
        self.local.entry(link).or_default().extend(channel);
    }

    pub fn local_closed(&mut self, link: LinkId) {
        self.local.remove(&link);
        self.local_peers.retain(|_, l| *l != link);
    }

    /// The discovery tokens of every channel: `(channel, token)` for the current slot and, when
    /// `with_previous`, the one before (a peer whose clock is a little behind).
    pub fn tokens(&self, now_ms: u64, with_previous: bool) -> Vec<(ChannelId, [u8; 16])> {
        let slot = now_ms / TOKEN_SLOT_MS;
        let mut out = Vec::new();
        for (id, (ch, _)) in &self.channels {
            out.push((*id, ch.token(slot)));
            if with_previous && slot > 0 {
                out.push((*id, ch.token(slot - 1)));
            }
        }
        out.sort();
        out
    }

    /// An envelope from a nearby link: `topic(32) || envelope`.
    pub fn open_local(&mut self, link: LinkId, data: &[u8]) -> Option<(ChannelId, Envelope)> {
        if data.len() < 32 {
            return None;
        }
        let topic: [u8; 32] = data[..32].try_into().ok()?;
        let id = *self.topics.get(&topic)?;
        let (ch, _) = self.channels.get_mut(&id)?;
        let sealed = &data[32..];
        let env = envelope::open(&ch.key, &topic, sealed).ok()?;
        // Envelopes are public on the relays: anyone nearby can replay one. Only a first arrival
        // says its sender may be on this link (a copy the relay brought first proves nothing).
        ch.guard.check(sealed, &env, now_ms(), envelope::MAX_AGE_MS).ok()?;
        self.local.entry(link).or_default().insert(id);
        if env.from != self.tag {
            self.local_peers.insert((id, env.from), link);
        }
        if env.from == self.tag || (env.to != envelope::BROADCAST && env.to != self.tag) {
            return None;
        }
        Some((id, env))
    }

    fn broadcast(&self, cmd: OutCmd) {
        for o in &self.outs {
            let _ = o.send(cmd.clone());
        }
    }

    pub fn add(&mut self, ch: Channel) -> ChannelId {
        let id = self.next_id;
        self.next_id += 1;
        let topics = vec![(ch.topic)(self.day), (ch.topic)(self.day - 1)];
        for t in &topics {
            self.topics.insert(*t, id);
            self.broadcast(OutCmd::Join(*t));
        }
        self.channels.insert(id, (ch, topics));
        id
    }

    pub fn remove(&mut self, id: ChannelId) {
        for chans in self.local.values_mut() {
            chans.remove(&id);
        }
        self.local_peers.retain(|(ch, _), _| *ch != id);
        if let Some((_, topics)) = self.channels.remove(&id) {
            for t in topics {
                self.topics.remove(&t);
                self.broadcast(OutCmd::Leave(t));
            }
        }
    }

    /// Follows UTC midnight: call every few minutes.
    pub fn rotate(&mut self) {
        let day = hush_core::utc_day(now_ms());
        if day == self.day {
            return;
        }
        self.day = day;
        let ids: Vec<ChannelId> = self.channels.keys().copied().collect();
        for id in ids {
            let Some((ch, old)) = self.channels.get_mut(&id) else {
                continue;
            };
            let new = vec![(ch.topic)(day), (ch.topic)(day - 1)];
            let gone: Vec<[u8; 32]> = old.iter().filter(|t| !new.contains(t)).copied().collect();
            let added: Vec<[u8; 32]> = new.iter().filter(|t| !old.contains(t)).copied().collect();
            *old = new;
            for t in gone {
                self.topics.remove(&t);
                self.broadcast(OutCmd::Leave(t));
            }
            for t in added {
                self.topics.insert(t, id);
                self.broadcast(OutCmd::Join(t));
            }
        }
    }

    /// Seals and sends; returns the manual code (`hushcode:…`) of the envelope.
    pub fn send(&self, id: ChannelId, kind: u8, body: &[u8], to: Option<PeerTag>) -> Option<String> {
        let (ch, _) = self.channels.get(&id)?;
        let topic = (ch.topic)(hush_core::utc_day(now_ms()));
        let env = Envelope {
            ts_ms: now_ms(),
            from: self.tag,
            to: to.unwrap_or(envelope::BROADCAST),
            kind,
            body: body.to_vec(),
        };
        let sealed = envelope::seal(&ch.key, &topic, &env, &mut crate::util::rng());
        for (link, chans) in &self.local {
            if chans.contains(&id) {
                let mut payload = topic.to_vec();
                payload.extend_from_slice(&sealed);
                let _ = self.local_out.send((*link, payload));
            }
        }
        let wire = b64u(&sealed);
        self.broadcast(OutCmd::Send {
            topic,
            wire: wire.clone(),
        });
        Some(format!("hushcode:{wire}"))
    }

    fn open(&mut self, wire: &str, topic: [u8; 32], manual: bool) -> Option<(ChannelId, Envelope)> {
        let id = *self.topics.get(&topic)?;
        let (ch, _) = self.channels.get_mut(&id)?;
        let data = from_b64u(wire)?;
        let env = envelope::open(&ch.key, &topic, &data).ok()?;
        let max_age = if manual {
            envelope::MANUAL_MAX_AGE_MS
        } else {
            envelope::MAX_AGE_MS
        };
        ch.guard.check(&data, &env, now_ms(), max_age).ok()?;
        if env.from == self.tag || (env.to != envelope::BROADCAST && env.to != self.tag) {
            return None;
        }
        Some((id, env))
    }

    /// A pasted code, opened against every channel we listen on.
    pub fn open_manual(&mut self, code: &str) -> Option<(ChannelId, Envelope)> {
        let s: String = code.split_whitespace().collect();
        let wire = s.strip_prefix("hushcode:")?.to_string();
        let topics: Vec<[u8; 32]> = self.topics.keys().copied().collect();
        topics.into_iter().find_map(|t| self.open(&wire, t, true))
    }

    /// The next envelope for us, from someone else.
    pub async fn next(&mut self) -> Option<(ChannelId, Envelope)> {
        loop {
            let (wire, topic) = self.rx.recv().await?;
            // The whole envelope, hashed: keyed on a prefix, a relay could send a corrupted copy
            // first and get the genuine one dropped as a duplicate.
            let mut key = topic[..2].to_vec();
            key.extend_from_slice(&sha2::Sha256::digest(wire.as_bytes())[..16]);
            if !self.seen.insert(key) {
                continue;
            }
            if self.seen.len() > 4096 {
                self.seen.clear();
            }
            if let Some(hit) = self.open(&wire, topic, false) {
                return Some(hit);
            }
        }
    }
}
