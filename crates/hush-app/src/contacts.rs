//! Saved contacts, identifiers and the vault, as `web/src/app/contacts.ts` and `store.ts` handle
//! them in the browser:
//! - **saving**: in a conversation where both sides compared the emojis, each sends its contact
//!   card (its signed bundle, bound to this session); each stores the other's card and the
//!   session's contact secret;
//! - **reconnecting**: each contact has a rendezvous derived from that secret. Whoever comes
//!   online posts HERE; the side with the smaller identifier hash starts the contact handshake
//!   (`KKpsk0 + X-Wing`), so the two never race. Nearby, the same happens over Bluetooth or
//!   Wi-Fi;
//! - **identifiers**: when reachable, we answer lookups of our identifier with our bundle, and
//!   show incoming requests (each carrying a proof of work) until the user accepts or declines.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use hush_core::bundle::{self, Bundle};
use hush_core::contact::ContactKeys;
use hush_core::envelope::{BROADCAST, Envelope, PeerTag, kind};
use hush_core::handshake::{
    ContactInitiator, ContactInitiatorReady, ContactParams, ContactResponder, ContactResponderPending, Established,
};
use hush_core::identity::Identity;
use hush_core::lookup::LookupKeys;
use hush_core::{id, pow};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use crate::api::{ChatId, ChatKind, ChatStatus, ContactInfo, EndReason, Event, LookupState, Notice, RequestInfo};
use crate::engine::{Chat, Engine, Internal, PENDING_TIMEOUT, Reply, RtcKey, RtcTx};
use crate::proto;
use crate::rtc::{self, RtcCmd};
use crate::signal::{Channel, ChannelId, LinkId, Via};
use crate::store::Contact;
use crate::util::{b64u, from_b64u, now_ms, rng};

const HERE_EVERY: Duration = Duration::from_secs(30);
const REQUEST_WAIT: Duration = Duration::from_secs(5 * 60);
const LOOKUP_EVERY: Duration = Duration::from_secs(4);
const LOOKUP_TRIES: u32 = 15;
const MAX_REQUESTS: usize = 20;
/// How long a request's proof of work is accepted (today's or yesterday's).
const REQUEST_REMEMBERED: Duration = Duration::from_secs(48 * 3600);
/// Requests remembered at most (each cost its sender a proof of work).
const MAX_REQUESTED: usize = 4096;

/// Vault, contact and identifier commands.
pub enum Cmd {
    CreateVault {
        passphrase: String,
        reply: Reply<()>,
    },
    UnlockVault {
        passphrase: String,
        reply: Reply<()>,
    },
    LockVault,
    EraseVault {
        reply: Reply<()>,
    },
    ChangePassphrase {
        passphrase: String,
        reply: Reply<()>,
    },
    ExportBackup {
        passphrase: String,
        reply: Reply<Vec<u8>>,
    },
    ImportBackup {
        file: Vec<u8>,
        passphrase: String,
        reply: Reply<usize>,
    },
    Remove {
        key: String,
    },
    SetHistory {
        key: String,
        on: bool,
    },
    /// Our own name for a contact (their cards never change it).
    Rename {
        key: String,
        name: String,
    },
    /// The open conversation with a contact, if any; says HERE so it connects when they are online.
    ChatWith {
        key: String,
        reply: Reply<Option<ChatId>>,
    },
    /// Our identifier, created if needed (needs an unlocked vault).
    EnsureIdentity {
        reply: Reply<String>,
    },
    /// What sealing a drop to people needs: the signed prekeys `(spk, epoch)` of these contacts
    /// (by key), and ours when `me`; and our seed, which signs it (and opens drops sealed to us).
    SealKeys {
        contacts: Vec<String>,
        me: bool,
        reply: Reply<SealKeys>,
    },
    SetReachable(bool),
    AddById {
        id: String,
    },
    Accept {
        id: String,
    },
    Decline {
        id: String,
    },
}

/// Keys for a drop sealed to people (see [`Cmd::SealKeys`]).
pub struct SealKeys {
    pub keys: Vec<(Vec<u8>, u32)>,
    pub seed: [u8; 32],
}

/// One contact handshake in flight, by the chat it will open.
enum Hs {
    /// We sent C1 (or a request); waiting for C2.
    Initiating {
        key: String,
        to: PeerTag,
        channel: ChannelId,
        state: ContactInitiator,
        name: String,
        /// A first request: C1 waits for its proof of work.
        request: Option<Vec<u8>>,
        since: Instant,
    },
    /// We read C2; waiting for our WebRTC answer.
    Answering {
        key: String,
        to: PeerTag,
        channel: ChannelId,
        state: ContactInitiatorReady,
        name: String,
        rtc: RtcTx,
        since: Instant,
    },
    /// We read C1; waiting for our WebRTC offer.
    Offering {
        key: String,
        from: PeerTag,
        channel: ChannelId,
        state: ContactResponderPending,
        name: String,
        bundle: Bundle,
        rtc: RtcTx,
        since: Instant,
    },
    /// We sent C2; waiting for C3.
    AwaitingC3 {
        key: String,
        from: PeerTag,
        channel: ChannelId,
        state: ContactResponder,
        name: String,
        rtc: Option<RtcTx>,
        since: Instant,
    },
}

impl Hs {
    fn key(&self) -> &str {
        match self {
            Hs::Initiating { key, .. }
            | Hs::Answering { key, .. }
            | Hs::Offering { key, .. }
            | Hs::AwaitingC3 { key, .. } => key,
        }
    }

    fn since(&self) -> Instant {
        match self {
            Hs::Initiating { since, .. }
            | Hs::Answering { since, .. }
            | Hs::Offering { since, .. }
            | Hs::AwaitingC3 { since, .. } => *since,
        }
    }

    fn timeout(&self) -> Duration {
        match self {
            Hs::Initiating { request: Some(_), .. } => REQUEST_WAIT,
            Hs::Initiating { .. } if self.key().starts_with("req:") => REQUEST_WAIT,
            _ => PENDING_TIMEOUT,
        }
    }

    fn rtc(self) -> Option<RtcTx> {
        match self {
            Hs::Answering { rtc, .. } | Hs::Offering { rtc, .. } => Some(rtc),
            Hs::AwaitingC3 { rtc, .. } => rtc,
            Hs::Initiating { .. } => None,
        }
    }
}

struct Request {
    id: String,
    name: String,
    note: String,
    bundle: Bundle,
    from: PeerTag,
    pending: ContactResponderPending,
    at: Instant,
}

struct Lookup {
    channel: ChannelId,
    tries: u32,
    last: Instant,
    found: bool,
}

#[derive(Default)]
pub(crate) struct State {
    /// Contact key → its rendezvous channel, and back.
    listening: HashMap<String, ChannelId>,
    by_channel: HashMap<ChannelId, String>,
    online: HashSet<String>,
    /// Contact key (or `req:<id>` for someone met by identifier) → their live chat.
    chats: HashMap<String, ChatId>,
    last_here: Option<Instant>,
    hs: HashMap<ChatId, Hs>,
    /// Our identifier's channel while reachable.
    reach: Option<ChannelId>,
    last_lookup_reply: Option<Instant>,
    requests: Vec<Request>,
    /// Every request read in the last two days, by the hash of its C1: anyone can seal one again
    /// on our public channel, and its proof of work stays valid that long. A declined request
    /// does not come back.
    requested: HashMap<[u8; 32], Instant>,
    lookups: HashMap<String, Lookup>,
}

/// Prekey epochs we accept: any up to next week's (prekeys derive from the seed).
fn epoch_window(now: u64) -> impl Fn(u32) -> bool {
    let cur = Identity::spk_epoch(hush_core::utc_day(now));
    move |e| e <= cur + 1
}

/// An identifier's channel: its key derives from the identifier, so anyone can seal on it.
fn lookup_channel(id_hash: &[u8; 32]) -> Channel {
    let keys = LookupKeys::new(id_hash);
    Channel::public(keys.env_key(), move |d| keys.topic(d))
}

fn hello_json(name: &str, sdp: Option<&str>) -> Vec<u8> {
    proto::hello(name, sdp)
}

impl Engine {
    fn me(&self) -> Option<&Identity> {
        self.store.identity()
    }

    fn my_key(&self) -> Option<String> {
        self.me().map(|m| hex::encode(m.public().id_hash()))
    }

    pub(crate) fn contacts_owns(&self, key: &RtcKey) -> bool {
        self.contacts.hs.contains_key(&key.0)
    }

    fn busy(&self, key: &str) -> bool {
        self.contacts
            .hs
            .values()
            .any(|h| h.key() == key && h.since().elapsed() < PENDING_TIMEOUT)
    }

    fn connected(&self, key: &str) -> bool {
        self.contacts
            .chats
            .get(key)
            .and_then(|c| self.chats.get(c))
            .is_some_and(Chat::connected)
    }

    pub(crate) fn emit_contacts(&self) {
        let list = self
            .store
            .contacts()
            .iter()
            .map(|c| ContactInfo {
                key: c.key.clone(),
                id: c.id.clone(),
                name: c.name.clone(),
                history: c.history,
                online: self.contacts.online.contains(&c.key),
                chat: self
                    .contacts
                    .chats
                    .get(&c.key)
                    .copied()
                    .filter(|id| self.chats.contains_key(id)),
            })
            .collect();
        self.emit(Event::Contacts(list));
    }

    fn emit_requests(&self) {
        self.emit(Event::Requests(
            self.contacts
                .requests
                .iter()
                .map(|r| RequestInfo {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    note: r.note.clone(),
                })
                .collect(),
        ));
    }

    fn emit_vault(&self) {
        self.emit(Event::Vault(self.store.state()));
        self.emit(Event::Identity(self.me().map(|m| m.public().id())));
        self.emit_contacts();
    }

    // ── vault ───────────────────────────────────────────────────────────────

    pub(crate) async fn on_contacts_command(&mut self, c: Cmd) {
        match c {
            Cmd::CreateVault { passphrase, reply } => {
                let r = self.store.create(&passphrase);
                if r.is_ok() {
                    self.after_unlock();
                }
                let _ = reply.send(r);
            }
            Cmd::UnlockVault { passphrase, reply } => {
                let r = self.store.unlock(&passphrase);
                if r.is_ok() {
                    self.after_unlock();
                }
                let _ = reply.send(r);
            }
            Cmd::LockVault => {
                self.stop_contacts();
                self.store.lock();
                self.emit_vault();
            }
            Cmd::EraseVault { reply } => {
                self.stop_contacts();
                let _ = reply.send(self.store.erase());
                self.emit_vault();
            }
            Cmd::ChangePassphrase { passphrase, reply } => {
                let _ = reply.send(self.store.change_passphrase(&passphrase));
            }
            Cmd::ExportBackup { passphrase, reply } => {
                let _ = reply.send(self.store.export_backup(&passphrase));
            }
            Cmd::ImportBackup {
                file,
                passphrase,
                reply,
            } => {
                let r = self.store.import_backup(&file, &passphrase);
                if r.is_ok() {
                    self.after_unlock();
                }
                let _ = reply.send(r);
            }
            Cmd::Remove { key } => {
                let _ = self.store.remove_contact(&key);
                if let Some(chat) = self.contacts.chats.get(&key).copied() {
                    self.end_chat(chat, EndReason::Ended);
                }
                self.watch();
                self.emit_contacts();
            }
            Cmd::SetHistory { key, on } => {
                if let Some(c) = self.store.contact(&key).cloned() {
                    let _ = self.store.save_contact(Contact { history: on, ..c });
                    if !on {
                        let _ = self.store.clear_history(&key);
                    }
                    self.emit_contacts();
                }
            }
            Cmd::Rename { key, name } => {
                let name = crate::text::clean_name(&name);
                if let Some(c) = self.store.contact(&key).cloned()
                    && !name.is_empty()
                {
                    let _ = self.store.save_contact(Contact { name, ..c });
                    self.emit_contacts();
                }
            }
            Cmd::ChatWith { key, reply } => {
                let live = self
                    .contacts
                    .chats
                    .get(&key)
                    .copied()
                    .filter(|c| self.chats.get(c).is_some_and(|c| !c.ended));
                if live.is_none() {
                    self.here(&key, None);
                }
                let _ = reply.send(Ok(live));
            }
            Cmd::EnsureIdentity { reply } => {
                let name = self.name();
                let r = self.store.ensure_identity(&name).map(|me| me.public().id());
                if r.is_ok() {
                    self.watch();
                    self.emit(Event::Identity(self.me().map(|m| m.public().id())));
                }
                let _ = reply.send(r);
            }
            Cmd::SealKeys { contacts, me, reply } => {
                let r = (|| {
                    let name = self.name();
                    let seed = *self.store.ensure_identity(&name)?.seed();
                    let mut keys = Vec::new();
                    for key in &contacts {
                        let c = self.store.contact(key).ok_or_else(|| anyhow!("not a contact"))?;
                        let b = crate::util::from_b64u(&c.bundle)
                            .and_then(|b| hush_core::bundle::Bundle::parse_stored(&b).ok())
                            .ok_or_else(|| anyhow!("the contact's keys do not verify"))?;
                        keys.push((b.spk, b.spk_epoch));
                    }
                    if me {
                        let id = hush_core::identity::Identity::from_seed(seed);
                        let epoch = hush_core::identity::Identity::spk_epoch(hush_core::utc_day(now_ms()));
                        keys.push((id.spk(epoch).public(), epoch));
                    }
                    Ok(SealKeys { keys, seed })
                })();
                let _ = reply.send(r);
            }
            Cmd::SetReachable(on) => self.set_reachable(on),
            Cmd::AddById { id } => self.add_by_id(&id),
            Cmd::Accept { id } => self.accept_request(&id),
            Cmd::Decline { id } => {
                self.contacts.requests.retain(|r| r.id != id);
                self.emit_requests();
            }
        }
    }

    fn after_unlock(&mut self) {
        self.watch();
        self.emit_vault();
    }

    /// Stops everything that needs the vault: contact chats, presence, reachability.
    fn stop_contacts(&mut self) {
        let chats: Vec<ChatId> = self.contacts.chats.values().copied().collect();
        for c in chats {
            self.end_chat(c, EndReason::Ended);
        }
        for (_, ch) in self.contacts.listening.drain() {
            self.rdv.remove(ch);
        }
        self.contacts.by_channel.clear();
        self.contacts.online.clear();
        self.set_reachable(false);
        for (_, l) in self.contacts.lookups.drain() {
            self.rdv.remove(l.channel);
        }
        self.contacts.requests.clear();
        self.refresh_discovery();
    }

    /// Listens on every saved contact's rendezvous (and stops for removed ones).
    pub(crate) fn watch(&mut self) {
        if !self.store.is_unlocked() || self.me().is_none() {
            return;
        }
        let keys: HashSet<String> = self.store.contacts().iter().map(|c| c.key.clone()).collect();
        let gone: Vec<String> = self
            .contacts
            .listening
            .keys()
            .filter(|k| !keys.contains(*k))
            .cloned()
            .collect();
        for k in gone {
            if let Some(ch) = self.contacts.listening.remove(&k) {
                self.rdv.remove(ch);
                self.contacts.by_channel.remove(&ch);
            }
        }
        let new: Vec<Contact> = self
            .store
            .contacts()
            .iter()
            .filter(|c| !self.contacts.listening.contains_key(&c.key))
            .cloned()
            .collect();
        for c in new {
            let Some(secret) = from_b64u(&c.secret).and_then(|s| <[u8; 32]>::try_from(s.as_slice()).ok()) else {
                continue;
            };
            let keys = ContactKeys::new(&secret);
            let env = keys.env_key();
            let ch = self.rdv.add(Channel::new(env, move |d| keys.topic(d)));
            self.contacts.listening.insert(c.key.clone(), ch);
            self.contacts.by_channel.insert(ch, c.key.clone());
            self.here(&c.key, None);
        }
        self.refresh_discovery();
    }

    fn here(&self, key: &str, to: Option<PeerTag>) {
        let (Some(me), Some(ch)) = (self.me(), self.contacts.listening.get(key)) else {
            return;
        };
        self.rdv.send(*ch, kind::HERE, me.public().id_hash(), to);
    }

    pub(crate) fn contacts_tick(&mut self) {
        let due = self.contacts.last_here.is_none_or(|t| t.elapsed() >= HERE_EVERY);
        if due {
            self.contacts.last_here = Some(Instant::now());
            let keys: Vec<String> = self.contacts.listening.keys().cloned().collect();
            for k in keys {
                if !self.connected(&k) {
                    self.here(&k, None);
                }
            }
        }
        let stale: Vec<ChatId> = self
            .contacts
            .hs
            .iter()
            .filter(|(_, h)| h.since().elapsed() >= h.timeout())
            .map(|(c, _)| *c)
            .collect();
        for chat in stale {
            if let Some(r) = self.contacts.hs.remove(&chat).and_then(Hs::rtc) {
                let _ = r.send(RtcCmd::Close);
            }
            self.end_chat(chat, EndReason::Failed);
        }
        let before = self.contacts.requests.len();
        self.contacts.requests.retain(|r| r.at.elapsed() < REQUEST_WAIT);
        if self.contacts.requests.len() != before {
            self.emit_requests();
        }
        let ids: Vec<String> = self.contacts.lookups.keys().cloned().collect();
        for id in ids {
            let Some(l) = self.contacts.lookups.get_mut(&id) else {
                continue;
            };
            if l.found || l.last.elapsed() < LOOKUP_EVERY {
                continue;
            }
            if l.tries >= LOOKUP_TRIES {
                let ch = l.channel;
                self.contacts.lookups.remove(&id);
                self.rdv.remove(ch);
                self.emit(Event::Lookup {
                    id,
                    state: LookupState::NotFound,
                    chat: None,
                });
                continue;
            }
            l.tries += 1;
            l.last = Instant::now();
            let ch = l.channel;
            self.rdv.send(ch, kind::LOOKUP, &[], None);
        }
    }

    pub(crate) fn contacts_chat_ended(&mut self, chat: ChatId) {
        if let Some(r) = self.contacts.hs.remove(&chat).and_then(Hs::rtc) {
            let _ = r.send(RtcCmd::Close);
        }
        let keys: Vec<String> = self
            .contacts
            .chats
            .iter()
            .filter(|(_, c)| **c == chat)
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys {
            self.contacts.chats.remove(&k);
            self.contacts.online.remove(&k);
            // A conversation with someone found by identifier: its lookup is over.
            if let Some(l) = k.strip_prefix("req:").and_then(|id| self.contacts.lookups.remove(id)) {
                self.rdv.remove(l.channel);
            }
        }
        self.emit_contacts();
    }

    /// Saved contacts to find nearby: `(channel, dial)` where `dial` says we look for them
    /// (we start handshakes with them) rather than advertise.
    pub(crate) fn contact_discovery(&self) -> Vec<(ChannelId, bool)> {
        let Some(mine) = self.my_key() else { return Vec::new() };
        self.contacts
            .listening
            .iter()
            .filter(|(k, _)| !self.connected(k))
            .map(|(k, ch)| (*ch, mine.as_str() < k.as_str()))
            .collect()
    }

    /// We dialed a nearby device advertising a contact's token: say we are here, on it too.
    pub(crate) fn contact_dialed(&mut self, ch: ChannelId) {
        if let Some(key) = self.contacts.by_channel.get(&ch).cloned() {
            self.here(&key, None);
        }
    }

    // ── envelopes ───────────────────────────────────────────────────────────

    pub(crate) fn on_contact_envelope(&mut self, ch: ChannelId, env: Envelope, via: Via) {
        let to_us = env.to != BROADCAST;
        if let Some(key) = self.contacts.by_channel.get(&ch).cloned() {
            match env.kind {
                kind::HERE => self.on_here(&key, &env),
                kind::C1 if to_us => self.on_c1(&key, ch, &env, via),
                kind::C2 if to_us => self.on_c2(ch, &env, via),
                kind::C3 if to_us => self.on_c3(ch, &env),
                _ => {}
            }
            return;
        }
        if Some(ch) == self.contacts.reach {
            match env.kind {
                kind::LOOKUP => self.on_lookup(ch, &env),
                kind::REQUEST if to_us => self.on_request(&env),
                kind::C3 if to_us => self.on_c3(ch, &env),
                _ => {}
            }
            return;
        }
        if let Some(id) = self
            .contacts
            .lookups
            .iter()
            .find(|(_, l)| l.channel == ch)
            .map(|(i, _)| i.clone())
        {
            match env.kind {
                kind::BUNDLE if to_us => self.on_bundle(&id, ch, &env),
                kind::C2 if to_us => self.on_c2(ch, &env, via),
                _ => {}
            }
        }
    }

    fn on_here(&mut self, key: &str, env: &Envelope) {
        if hex::encode(&env.body) != key {
            return;
        }
        if self.contacts.online.insert(key.to_string()) {
            self.emit_contacts();
        }
        if self.connected(key) || self.busy(key) {
            return;
        }
        let Some(mine) = self.my_key() else { return };
        if mine.as_str() < key {
            self.initiate(key.to_string(), env.from, None);
        } else if env.to == BROADCAST {
            // Let them know we are here: they start.
            self.here(key, Some(env.from));
        }
    }

    fn open_contact_chat(&mut self, key: &str, kind: ChatKind) -> ChatId {
        if let Some(old) = self.contacts.chats.get(key).copied() {
            self.end_chat(old, EndReason::Ended);
        }
        let mut chat = Chat::new(kind.clone(), false);
        chat.ended = false;
        let id = self.new_chat(chat);
        self.contacts.chats.insert(key.to_string(), id);
        self.emit(Event::ChatOpened {
            chat: id,
            kind,
            url: None,
            expires_ms: None,
        });
        self.emit(Event::ChatStatus {
            chat: id,
            status: ChatStatus::Connecting,
        });
        self.emit_contacts();
        id
    }

    /// Initiator side. `first`: a first request to an identifier, `(peer bundle, channel)`.
    fn initiate(&mut self, key: String, to: PeerTag, first: Option<(Bundle, ChannelId)>) {
        let Some(me) = self.me() else { return };
        let my_hash = *me.public().id_hash();
        let name = self.name();
        let (peer, psk, channel, payload, kind) = match &first {
            Some((b, ch)) => {
                let mine = Bundle::create(me, &name, now_ms(), &mut rng());
                let payload = json!({"v": 1, "name": name, "bundle": b64u(&mine.bytes)})
                    .to_string()
                    .into_bytes();
                (
                    b.clone(),
                    LookupKeys::new(b.id_hash()).psk(),
                    *ch,
                    payload,
                    ChatKind::Stranger { id: b.identity.id() },
                )
            }
            None => {
                let Some(c) = self.store.contact(&key) else { return };
                let Some(peer) = from_b64u(&c.bundle).and_then(|b| Bundle::parse_stored(&b).ok()) else {
                    return;
                };
                let Some(secret) = from_b64u(&c.secret).and_then(|s| <[u8; 32]>::try_from(s.as_slice()).ok()) else {
                    return;
                };
                let Some(ch) = self.contacts.listening.get(&key).copied() else {
                    return;
                };
                (
                    peer,
                    ContactKeys::new(&secret).psk(),
                    ch,
                    hello_json(&name, None),
                    ChatKind::Contact { key: key.clone() },
                )
            }
        };
        let params = ContactParams {
            psk: &psk,
            ids: (&my_hash, peer.id_hash()),
        };
        let Ok((state, c1)) = ContactInitiator::start(&params, &peer.spk, peer.spk_epoch, &payload, &mut rng()) else {
            return;
        };
        let chat = self.open_contact_chat(&key, kind);
        let request = first.is_some().then(|| c1.clone());
        self.contacts.hs.insert(
            chat,
            Hs::Initiating {
                key: key.clone(),
                to,
                channel,
                state,
                name: peer.name.clone(),
                request,
                since: Instant::now(),
            },
        );
        match first {
            Some((b, _)) => {
                // The proof of work takes about a second: off the actor.
                let challenge = LookupKeys::new(b.id_hash()).challenge(hush_core::utc_day(now_ms()), &c1);
                let internal = self.internal_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let nonce = pow::solve(&challenge, pow::DEFAULT_BITS);
                    let _ = internal.send(Internal::Pow { key, nonce });
                });
            }
            None => {
                self.rdv.send(channel, kind::C1, &c1, Some(to));
            }
        }
    }

    pub(crate) fn request_pow_ready(&mut self, key: &str, nonce: u64) {
        let Some(me) = self.me() else { return };
        let my_hash = *me.public().id_hash();
        let found = self.contacts.hs.values_mut().find_map(|h| match h {
            Hs::Initiating {
                key: k,
                to,
                channel,
                request,
                ..
            } if k == key => request.take().map(|c1| (*to, *channel, c1)),
            _ => None,
        });
        if let Some((to, channel, c1)) = found {
            let mut body = my_hash.to_vec();
            body.extend_from_slice(&nonce.to_be_bytes());
            body.extend_from_slice(&c1);
            self.rdv.send(channel, kind::REQUEST, &body, Some(to));
        }
    }

    fn on_c2(&mut self, ch: ChannelId, env: &Envelope, via: Via) {
        let Some(chat) = self.contacts.hs.iter().find_map(|(c, h)| match h {
            Hs::Initiating {
                channel,
                to,
                request: None,
                ..
            } if *channel == ch && *to == env.from => Some(*c),
            _ => None,
        }) else {
            return;
        };
        let Some(Hs::Initiating {
            key,
            to,
            channel,
            state,
            name,
            ..
        }) = self.contacts.hs.remove(&chat)
        else {
            return;
        };
        let Some(me) = self.store.identity() else { return };
        let Ok((ready, payload)) = state.read_c2(me, epoch_window(now_ms()), &env.body) else {
            self.end_chat(chat, EndReason::Failed);
            return;
        };
        let hello = proto::parse_hello(&payload).unwrap_or_default();
        let name = if hello.name.is_empty() { name } else { hello.name };
        // Without an offer, the session runs on the nearby link this C2 came over.
        match (hello.sdp, via.link()) {
            (None, Some(_)) => {
                let (c3, est) = ready.write_c3(&hello_json(&self.name(), None));
                self.rdv.send(channel, kind::C3, &c3, Some(to));
                self.contact_established(chat, &key, to, &est, name, None, channel);
            }
            (Some(sdp), _) => {
                let rtc = rtc::spawn((chat, to), self.rtc_opts(), self.rtc_tx_clone());
                let (tx, rx) = oneshot::channel();
                let _ = rtc.send(RtcCmd::AcceptOffer(sdp, tx));
                self.contacts.hs.insert(
                    chat,
                    Hs::Answering {
                        key,
                        to,
                        channel,
                        state: ready,
                        name,
                        rtc,
                        since: Instant::now(),
                    },
                );
                let internal = self.internal_tx.clone();
                tokio::spawn(async move {
                    let result = rx.await.unwrap_or_else(|_| Err(anyhow!("closed")));
                    let _ = internal.send(Internal::Answer {
                        key: (chat, to),
                        result,
                    });
                });
            }
            (None, None) => self.end_chat(chat, EndReason::Failed),
        }
    }

    pub(crate) fn contact_answer_ready(&mut self, (chat, _): RtcKey, result: Result<String>) {
        let Some(Hs::Answering {
            key,
            to,
            channel,
            state,
            name,
            rtc,
            ..
        }) = self.contacts.hs.remove(&chat)
        else {
            return;
        };
        let Ok(answer) = result else {
            let _ = rtc.send(RtcCmd::Close);
            self.end_chat(chat, EndReason::Failed);
            return;
        };
        let (c3, est) = state.write_c3(json!({"v": 1, "sdp": answer}).to_string().as_bytes());
        self.rdv.send(channel, kind::C3, &c3, Some(to));
        self.contact_established(chat, &key, to, &est, name, Some(rtc), channel);
    }

    fn on_c1(&mut self, key: &str, ch: ChannelId, env: &Envelope, via: Via) {
        if self.busy(key) || self.connected(key) {
            return;
        }
        let Some(c) = self.store.contact(key).cloned() else {
            return;
        };
        let Some(me) = self.me() else { return };
        let my_hash = *me.public().id_hash();
        let (Some(secret), Ok(peer_hash), Some(bundle)) = (
            from_b64u(&c.secret).and_then(|s| <[u8; 32]>::try_from(s.as_slice()).ok()),
            hex::decode(key)
                .map_err(|_| ())
                .and_then(|k| <[u8; 32]>::try_from(k.as_slice()).map_err(|_| ())),
            from_b64u(&c.bundle).and_then(|b| Bundle::parse_stored(&b).ok()),
        ) else {
            return;
        };
        let psk = ContactKeys::new(&secret).psk();
        let params = ContactParams {
            psk: &psk,
            ids: (&my_hash, &peer_hash),
        };
        let Ok((pending, payload)) = ContactResponderPending::read_c1(&params, me, epoch_window(now_ms()), &env.body)
        else {
            return;
        };
        let name = proto::parse_hello(&payload)
            .map(|h| h.name)
            .filter(|n| !n.is_empty())
            .unwrap_or(c.name);
        let chat = self.open_contact_chat(key, ChatKind::Contact { key: key.to_string() });
        self.respond(chat, key.to_string(), env.from, ch, pending, bundle, name, via.link());
    }

    /// Responder side, once C1 (or an accepted request) was read.
    #[allow(clippy::too_many_arguments)]
    fn respond(
        &mut self,
        chat: ChatId,
        key: String,
        from: PeerTag,
        channel: ChannelId,
        pending: ContactResponderPending,
        bundle: Bundle,
        name: String,
        local: Option<LinkId>,
    ) {
        // C1 came over a nearby link: no WebRTC. Only the link C1 itself used counts, since a
        // peer heard on a link before may be someone nearby replaying its relayed envelopes.
        if local.is_some() {
            match pending.write_c2(
                &bundle.spk,
                bundle.spk_epoch,
                &hello_json(&self.name(), None),
                &mut rng(),
            ) {
                Ok((state, c2)) => {
                    self.rdv.send(channel, kind::C2, &c2, Some(from));
                    self.contacts.hs.insert(
                        chat,
                        Hs::AwaitingC3 {
                            key,
                            from,
                            channel,
                            state,
                            name,
                            rtc: None,
                            since: Instant::now(),
                        },
                    );
                }
                Err(_) => self.end_chat(chat, EndReason::Failed),
            }
            return;
        }
        let rtc = rtc::spawn((chat, from), self.rtc_opts(), self.rtc_tx_clone());
        let (tx, rx) = oneshot::channel();
        let _ = rtc.send(RtcCmd::CreateOffer(tx));
        self.contacts.hs.insert(
            chat,
            Hs::Offering {
                key,
                from,
                channel,
                state: pending,
                name,
                bundle,
                rtc,
                since: Instant::now(),
            },
        );
        let internal = self.internal_tx.clone();
        tokio::spawn(async move {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("closed")));
            let _ = internal.send(Internal::Offer {
                key: (chat, from),
                result,
            });
        });
    }

    pub(crate) fn contact_offer_ready(&mut self, (chat, _): RtcKey, result: Result<String>) {
        let Some(Hs::Offering {
            key,
            from,
            channel,
            state,
            name,
            bundle,
            rtc,
            ..
        }) = self.contacts.hs.remove(&chat)
        else {
            return;
        };
        let written = result.and_then(|offer| {
            state
                .write_c2(
                    &bundle.spk,
                    bundle.spk_epoch,
                    &hello_json(&self.name(), Some(&offer)),
                    &mut rng(),
                )
                .map_err(|e| anyhow!("{e}"))
        });
        let Ok((state, c2)) = written else {
            let _ = rtc.send(RtcCmd::Close);
            self.end_chat(chat, EndReason::Failed);
            return;
        };
        self.rdv.send(channel, kind::C2, &c2, Some(from));
        self.contacts.hs.insert(
            chat,
            Hs::AwaitingC3 {
                key,
                from,
                channel,
                state,
                name,
                rtc: Some(rtc),
                since: Instant::now(),
            },
        );
    }

    fn on_c3(&mut self, ch: ChannelId, env: &Envelope) {
        let Some(chat) = self.contacts.hs.iter().find_map(|(c, h)| match h {
            Hs::AwaitingC3 { channel, from, .. } if *channel == ch && *from == env.from => Some(*c),
            _ => None,
        }) else {
            return;
        };
        let Some(Hs::AwaitingC3 {
            key,
            from,
            channel,
            state,
            name,
            rtc,
            ..
        }) = self.contacts.hs.remove(&chat)
        else {
            return;
        };
        let Ok((payload, est)) = state.read_c3(&env.body) else {
            if let Some(r) = rtc {
                let _ = r.send(RtcCmd::Close);
            }
            self.end_chat(chat, EndReason::Failed);
            return;
        };
        if let (Some(r), Some(sdp)) = (&rtc, proto::parse_hello(&payload).and_then(|h| h.sdp)) {
            let _ = r.send(RtcCmd::AcceptAnswer(sdp));
        }
        self.contact_established(chat, &key, from, &est, name, rtc, channel);
    }

    #[allow(clippy::too_many_arguments)]
    fn contact_established(
        &mut self,
        chat: ChatId,
        key: &str,
        tag: PeerTag,
        est: &Established,
        name: String,
        rtc: Option<RtcTx>,
        channel: ChannelId,
    ) {
        // Saved contacts were verified when saved; people met by identifier are authenticated
        // by it (the identifier hashes the keys this handshake proved).
        self.add_peer(chat, tag, est, name, rtc, Some(channel), true);
        let first = key.starts_with("req:");
        let name = self.name();
        let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(&tag)) else {
            return;
        };
        p.secret = Some(p.session.contact_secret());
        p.saving = first;
        let hash = *p.session.handshake_hash();
        // Our card: refreshes our prekey and name with a contact, saves us with a new one.
        if let Some(me) = self.store.identity() {
            let card = bundle::card(me, &name, &hash, now_ms(), &mut rng());
            self.send_app(
                chat,
                &tag,
                hush_core::session::lane::CTRL,
                json!({"t": "card", "card": b64u(&card)}),
            );
        }
        let history = self.store.contact(key).is_some_and(|c| c.history);
        if history && let Ok(past) = self.store.history(key) {
            let peer_name = self
                .chats
                .get(&chat)
                .and_then(|c| c.peers.get(&tag))
                .map(|p| p.name.clone())
                .unwrap_or_default();
            for e in past {
                self.emit(Event::Message {
                    chat,
                    peer: (!e.mine).then(|| crate::engine::tag_hex(&tag)),
                    id: e.id,
                    author: if e.mine { name.clone() } else { peer_name.clone() },
                    text: e.text,
                    ts: e.ts,
                    expires_at: e.expires_at,
                    past: true,
                });
            }
        }
        self.emit_contacts();
    }

    fn rtc_tx_clone(&self) -> tokio::sync::mpsc::UnboundedSender<(RtcKey, rtc::RtcEvent)> {
        self.rtc_sender()
    }

    // ── cards ───────────────────────────────────────────────────────────────

    pub(crate) fn on_card(&mut self, chat: ChatId, tag: &PeerTag, card: &str) {
        let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
            return;
        };
        let Some(info) =
            from_b64u(card).and_then(|c| bundle::verify_card(&c, p.session.handshake_hash(), now_ms()).ok())
        else {
            return;
        };
        p.secret.get_or_insert_with(|| p.session.contact_secret());
        let key = hex::encode(info.id_hash());
        let kind = self.chats[&chat].kind.clone();
        match kind {
            ChatKind::Contact { key: k } => {
                // A contact refreshing its bundle (new prekey). The name stays the one saved.
                if k == key
                    && let Some(c) = self.store.contact(&key).cloned()
                {
                    let _ = self.store.save_contact(Contact {
                        bundle: b64u(&info.bytes),
                        ..c
                    });
                }
            }
            ChatKind::Stranger { id } => {
                // The card of the identity this conversation was opened with, not another one
                // the same person holds.
                if info.identity.id() != id {
                    return;
                }
                if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) {
                    p.card = Some(info);
                }
                let saving = self.chats[&chat].peers.get(tag).is_some_and(|p| p.saving);
                if saving {
                    self.store_contact(chat, tag);
                }
            }
            ChatKind::Link { .. } => {
                if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) {
                    p.card = Some(info);
                }
                // Before the emojis match, the card could come from someone in the middle: it is
                // held, and only offered once both sides compared them.
                self.on_verified(chat, tag);
            }
        }
    }

    /// Both sides may now have compared the emojis: act on a held card.
    pub(crate) fn on_verified(&mut self, chat: ChatId, tag: &PeerTag) {
        let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) else {
            return;
        };
        if !p.verified() || p.card.is_none() {
            return;
        }
        if p.saving {
            self.store_contact(chat, tag);
            return;
        }
        if p.offered {
            return;
        }
        p.offered = true;
        let info = p.card.as_ref().expect("checked");
        let known = self.store.contact(&hex::encode(info.id_hash())).is_some();
        if !known {
            let (name, id) = (
                if info.name.is_empty() {
                    p.name.clone()
                } else {
                    info.name.clone()
                },
                info.identity.id(),
            );
            self.emit(Event::CardOffer {
                chat,
                peer: crate::engine::tag_hex(tag),
                name,
                id,
            });
        }
    }

    /// "Save contact": only with a verified peer and an unlocked vault.
    pub(crate) fn save_contact(&mut self, chat: ChatId, tag: PeerTag) {
        let Some(p) = self.chats.get(&chat).and_then(|c| c.peers.get(&tag)) else {
            return;
        };
        if !p.verified() {
            self.emit(Event::Notice {
                chat,
                notice: Notice::VerifyFirst,
            });
            return;
        }
        if !self.store.is_unlocked() {
            self.emit(Event::Notice {
                chat,
                notice: Notice::VaultNeeded,
            });
            return;
        }
        let name = self.name();
        let Ok(me) = self.store.ensure_identity(&name) else {
            return;
        };
        let me_public = me.public().clone();
        let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(&tag)) else {
            return;
        };
        p.secret.get_or_insert_with(|| p.session.contact_secret());
        p.saving = true;
        let hash = *p.session.handshake_hash();
        let has_card = p.card.is_some();
        let peer_name = p.name.clone();
        let card = bundle::card(
            self.store.identity().expect("ensured"),
            &name,
            &hash,
            now_ms(),
            &mut rng(),
        );
        let _ = me_public;
        self.send_app(
            chat,
            &tag,
            hush_core::session::lane::CTRL,
            json!({"t": "card", "card": b64u(&card)}),
        );
        self.emit(Event::Identity(self.me().map(|m| m.public().id())));
        if has_card {
            self.store_contact(chat, &tag);
        } else {
            self.emit(Event::Notice {
                chat,
                notice: Notice::ContactWaiting { name: peer_name },
            });
        }
    }

    fn store_contact(&mut self, chat: ChatId, tag: &PeerTag) {
        if !self.store.is_unlocked() || self.me().is_none() {
            return;
        }
        let Some(p) = self.chats.get(&chat).and_then(|c| c.peers.get(tag)) else {
            return;
        };
        if !p.verified() {
            return;
        }
        let (Some(info), Some(secret)) = (p.card.clone(), p.secret) else {
            return;
        };
        let key = hex::encode(info.id_hash());
        let existing = self.store.contact(&key).cloned();
        let name = existing
            .as_ref()
            .map(|c| c.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| (!info.name.is_empty()).then(|| info.name.clone()))
            .or_else(|| (!p.name.is_empty()).then(|| p.name.clone()))
            .unwrap_or_else(|| info.identity.id().chars().take(12).collect());
        let contact = Contact {
            id: info.identity.id(),
            key: key.clone(),
            name: name.clone(),
            bundle: b64u(&info.bytes),
            secret: b64u(&secret),
            added: existing.as_ref().map_or_else(now_ms, |c| c.added),
            history: existing.as_ref().is_some_and(|c| c.history),
        };
        if self.store.save_contact(contact).is_err() {
            return;
        }
        if let Some(p) = self.chats.get_mut(&chat).and_then(|c| c.peers.get_mut(tag)) {
            p.saving = false;
        }
        // The conversation now belongs to the contact: a later reconnection replaces it.
        if !self.contacts.chats.values().any(|c| *c == chat) {
            self.contacts.chats.insert(key.clone(), chat);
        } else if let Some(old) = self
            .contacts
            .chats
            .iter()
            .find(|(_, c)| **c == chat)
            .map(|(k, _)| k.clone())
            && old != key
        {
            self.contacts.chats.remove(&old);
            self.contacts.chats.insert(key.clone(), chat);
            // Found by identifier: that lookup is over.
            if let Some(l) = old.strip_prefix("req:").and_then(|id| self.contacts.lookups.remove(id)) {
                self.rdv.remove(l.channel);
            }
        }
        self.watch();
        if existing.is_none() {
            self.emit(Event::Notice {
                chat,
                notice: Notice::ContactSaved { name },
            });
        }
        self.emit_contacts();
    }

    // ── identifiers ─────────────────────────────────────────────────────────

    fn set_reachable(&mut self, on: bool) {
        if let Some(ch) = self.contacts.reach.take() {
            self.rdv.remove(ch);
        }
        let on = on && self.store.is_unlocked() && self.me().is_some();
        if on {
            let hash = *self.me().expect("checked").public().id_hash();
            self.contacts.reach = Some(self.rdv.add(lookup_channel(&hash)));
        }
        self.emit(Event::Reachable(on));
    }

    fn on_lookup(&mut self, ch: ChannelId, env: &Envelope) {
        // Each answer costs a hybrid signature: at most one a second.
        if self
            .contacts
            .last_lookup_reply
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.contacts.last_lookup_reply = Some(Instant::now());
        let name = self.name();
        let Some(me) = self.me() else { return };
        let b = Bundle::create(me, &name, now_ms(), &mut rng());
        self.rdv.send(ch, kind::BUNDLE, &b.bytes, Some(env.from));
    }

    fn on_request(&mut self, env: &Envelope) {
        if env.body.len() < 40 || self.contacts.requests.len() >= MAX_REQUESTS {
            return;
        }
        let Some(my_hash) = self.me().map(|m| *m.public().id_hash()) else {
            return;
        };
        let claimed: [u8; 32] = env.body[..32].try_into().expect("32 bytes");
        let nonce = u64::from_be_bytes(env.body[32..40].try_into().expect("8 bytes"));
        let c1 = &env.body[40..];
        // Before any costly check: a request read once is never read again.
        let seen: [u8; 32] = Sha256::digest(c1).into();
        if self.contacts.requested.contains_key(&seen) {
            return;
        }
        let lookup = LookupKeys::new(&my_hash);
        let day = hush_core::utc_day(now_ms());
        let ok = [day, day.saturating_sub(1)]
            .iter()
            .any(|d| pow::check(&lookup.challenge(*d, c1), nonce, pow::DEFAULT_BITS));
        if !ok {
            return;
        }
        let requested = &mut self.contacts.requested;
        requested.retain(|_, at| at.elapsed() < REQUEST_REMEMBERED);
        if requested.len() >= MAX_REQUESTED
            && let Some(oldest) = requested.iter().min_by_key(|(_, at)| **at).map(|(h, _)| *h)
        {
            requested.remove(&oldest);
        }
        requested.insert(seen, Instant::now());
        let psk = lookup.psk();
        let params = ContactParams {
            psk: &psk,
            ids: (&my_hash, &claimed),
        };
        let Some(me) = self.me() else { return };
        let Ok((pending, payload)) = ContactResponderPending::read_c1(&params, me, epoch_window(now_ms()), c1) else {
            return;
        };
        let Some(hello) = proto::parse_hello(&payload) else {
            return;
        };
        let Some(bundle) = hello
            .bundle
            .as_deref()
            .and_then(from_b64u)
            .and_then(|b| Bundle::parse(&b, now_ms()).ok())
        else {
            return;
        };
        // The bundle must be the requester's: C2 encapsulates to its prekey, so only its owner
        // can finish the handshake.
        if bundle.id_hash() != &claimed {
            return;
        }
        let id = bundle.identity.id();
        if self.contacts.requests.iter().any(|r| r.id == id) {
            return;
        }
        self.contacts.requests.push(Request {
            id,
            name: if hello.name.is_empty() {
                bundle.name.clone()
            } else {
                hello.name
            },
            note: hello.note.unwrap_or_default(),
            bundle,
            from: env.from,
            pending,
            at: Instant::now(),
        });
        self.emit_requests();
    }

    fn accept_request(&mut self, id: &str) {
        let Some(i) = self.contacts.requests.iter().position(|r| r.id == id) else {
            return;
        };
        let r = self.contacts.requests.remove(i);
        self.emit_requests();
        let Some(channel) = self.contacts.reach else { return };
        let key = format!("req:{}", r.id);
        let chat = self.open_contact_chat(&key, ChatKind::Stranger { id: r.id.clone() });
        let name = r.name.clone();
        self.respond(chat, key, r.from, channel, r.pending, r.bundle, name, None);
    }

    fn add_by_id(&mut self, input: &str) {
        let input = input.trim().to_lowercase();
        let Ok(hash) = id::decode(&input) else {
            self.emit(Event::Lookup {
                id: input,
                state: LookupState::NotFound,
                chat: None,
            });
            return;
        };
        if self.me().is_none() {
            let name = self.name();
            if self.store.ensure_identity(&name).is_err() {
                return;
            }
            self.emit(Event::Identity(self.me().map(|m| m.public().id())));
        }
        if self.contacts.lookups.contains_key(&input) {
            return;
        }
        let channel = self.rdv.add(lookup_channel(&hash));
        self.contacts.lookups.insert(
            input.clone(),
            Lookup {
                channel,
                tries: 1,
                last: Instant::now(),
                found: false,
            },
        );
        self.rdv.send(channel, kind::LOOKUP, &[], None);
        self.emit(Event::Lookup {
            id: input,
            state: LookupState::Searching,
            chat: None,
        });
    }

    fn on_bundle(&mut self, id: &str, ch: ChannelId, env: &Envelope) {
        let Some(l) = self.contacts.lookups.get_mut(id) else {
            return;
        };
        if l.found {
            return;
        }
        let Ok(b) = Bundle::parse(&env.body, now_ms()) else {
            return;
        };
        if b.identity.id() != id {
            return;
        }
        l.found = true;
        let key = format!("req:{id}");
        self.initiate(key.clone(), env.from, Some((b, ch)));
        let chat = self.contacts.chats.get(&key).copied();
        self.emit(Event::Lookup {
            id: id.to_string(),
            state: LookupState::Requested,
            chat,
        });
    }
}

/// Fails unless the input is a well-formed identifier (for the apps, before asking).
pub fn check_id(s: &str) -> Result<()> {
    if id::decode(s).is_err() {
        bail!("not a hush identifier");
    }
    Ok(())
}
