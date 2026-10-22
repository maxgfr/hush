//! WebAssembly bindings of `hush-core` for the web app.
//!
//! Every secret stays inside WebAssembly memory where it can: JavaScript gets handles (classes)
//! and ciphertext, and only sees plaintext it has to display. Errors surface as `Error` objects
//! whose message is the core error name (`Decrypt`, `Expired`, …).
#![forbid(unsafe_code)]

use getrandom::SysRng;
use hush_core::envelope::{self, Envelope, PeerTag, ReplayGuard};
use hush_core::handshake::{self as hs, ContactParams, Established};
use hush_core::identity::Identity as CoreIdentity;
use hush_core::link::{Link, LinkKeys, Mode};
use hush_core::vault::KdfParams;
use hush_core::{Error, backup, bundle, contact, id, lookup, pow, sas, session, vault};
use js_sys::{Array, Object, Reflect, Uint8Array};
use rand_core::UnwrapErr;
use wasm_bindgen::prelude::*;

fn rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

fn err(e: Error) -> JsError {
    JsError::new(&format!("{e:?}"))
}

fn now_ms(now: f64) -> u64 {
    if now.is_finite() && now > 0.0 { now as u64 } else { 0 }
}

fn set(o: &Object, k: &str, v: impl Into<JsValue>) {
    Reflect::set(o, &JsValue::from_str(k), &v.into()).expect("plain object");
}

fn bytes(b: &[u8]) -> Uint8Array {
    Uint8Array::from(b)
}

fn arr32(b: &[u8]) -> Result<[u8; 32], JsError> {
    b.try_into().map_err(|_| err(Error::Malformed))
}

fn tag(b: &[u8]) -> Result<PeerTag, JsError> {
    b.try_into().map_err(|_| err(Error::Malformed))
}

/// Prekeys derive from the seed, so any past epoch still decapsulates (a contact may hold an old
/// bundle); refuse only epochs from the future, beyond clock skew.
fn epoch_window(now: u64) -> impl Fn(u32) -> bool {
    let cur = CoreIdentity::spk_epoch(hush_core::utc_day(now));
    move |e| e <= cur + 1
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen(js_name = utcDay)]
pub fn utc_day(now: f64) -> u32 {
    hush_core::utc_day(now_ms(now))
}

#[wasm_bindgen(js_name = randomTag)]
pub fn random_tag() -> Vec<u8> {
    let mut t = [0u8; 16];
    rand_core::Rng::fill_bytes(&mut rng(), &mut t);
    t.to_vec()
}

// --------------------------------------------------------------------------------------------
// Links

/// A fresh link fragment (without `#`).
#[wasm_bindgen(js_name = linkNew)]
pub fn link_new(group: bool, ttl_min: u32, passphrase: bool, now: f64) -> String {
    let mode = if group { Mode::Group } else { Mode::Direct };
    Link::generate(mode, now_ms(now), u64::from(ttl_min), passphrase, &mut rng()).fragment()
}

/// `{ mode: "direct" | "group", expiresMs, passphrase, fragment }`, without deriving keys.
#[wasm_bindgen(js_name = linkInfo)]
pub fn link_info(input: &str) -> Result<Object, JsError> {
    let link = Link::parse(input).map_err(err)?;
    let o = Object::new();
    set(&o, "mode", if link.mode == Mode::Group { "group" } else { "direct" });
    set(&o, "expiresMs", (link.expires_min * 60_000) as f64);
    set(&o, "passphrase", link.passphrase);
    set(&o, "fragment", link.fragment());
    Ok(o)
}

/// Keys derived from a link (slow with a passphrase: Argon2id, 64 MiB).
#[wasm_bindgen]
pub struct LinkSecrets {
    keys: LinkKeys,
}

#[wasm_bindgen]
impl LinkSecrets {
    #[wasm_bindgen(constructor)]
    pub fn new(input: &str, passphrase: Option<String>) -> Result<LinkSecrets, JsError> {
        let link = Link::parse(input).map_err(err)?;
        let keys = link.keys(passphrase.as_deref()).map_err(err)?;
        Ok(Self { keys })
    }

    #[wasm_bindgen(getter)]
    pub fn group(&self) -> bool {
        self.keys.mode == Mode::Group
    }

    #[wasm_bindgen(getter, js_name = expiresMs)]
    pub fn expires_ms(&self) -> f64 {
        (self.keys.expires_min * 60_000) as f64
    }

    pub fn channel(&self) -> Channel {
        let keys = self.keys.clone();
        Channel::new(*self.keys.env_key, Box::new(move |d| keys.topic(d)))
    }

    /// Joiner: `{ state: LinkJoiner, hs1 }`.
    #[wasm_bindgen(js_name = joinerStart)]
    pub fn joiner_start(&self, payload: &[u8]) -> Object {
        let (state, hs1) = hs::LinkJoiner::start(&self.keys, payload, &mut rng());
        let o = Object::new();
        set(&o, "state", LinkJoiner { inner: state });
        set(&o, "hs1", bytes(&hs1));
        o
    }

    /// Creator: reads HS1, `{ state: LinkCreatorPending, payload }`.
    #[wasm_bindgen(js_name = creatorReadHs1)]
    pub fn creator_read_hs1(&self, hs1: &[u8]) -> Result<Object, JsError> {
        let (state, payload) = hs::LinkCreatorPending::read_hs1(&self.keys, hs1).map_err(err)?;
        let o = Object::new();
        set(&o, "state", LinkCreatorPending { inner: Some(state) });
        set(&o, "payload", bytes(&payload));
        Ok(o)
    }
}

#[wasm_bindgen]
pub struct LinkJoiner {
    inner: hs::LinkJoiner,
}

#[wasm_bindgen]
impl LinkJoiner {
    /// `{ state: LinkJoinerReady, payload }`. Can be called once per answering member.
    #[wasm_bindgen(js_name = readHs2)]
    pub fn read_hs2(&self, hs2: &[u8]) -> Result<Object, JsError> {
        let (state, payload) = self.inner.read_hs2(hs2).map_err(err)?;
        let o = Object::new();
        set(&o, "state", LinkJoinerReady { inner: Some(state) });
        set(&o, "payload", bytes(&payload));
        Ok(o)
    }
}

#[wasm_bindgen]
pub struct LinkJoinerReady {
    inner: Option<hs::LinkJoinerReady>,
}

#[wasm_bindgen]
impl LinkJoinerReady {
    /// `{ hs3, session, sas, hash }`.
    #[wasm_bindgen(js_name = writeHs3)]
    pub fn write_hs3(&mut self, payload: &[u8], now: f64) -> Result<Object, JsError> {
        let state = self.inner.take().ok_or(err(Error::BadState))?;
        let (hs3, est) = state.write_hs3(payload);
        let o = established(&est, now);
        set(&o, "hs3", bytes(&hs3));
        Ok(o)
    }
}

#[wasm_bindgen]
pub struct LinkCreatorPending {
    inner: Option<hs::LinkCreatorPending>,
}

#[wasm_bindgen]
impl LinkCreatorPending {
    /// `{ state: LinkCreator, hs2 }`.
    #[wasm_bindgen(js_name = writeHs2)]
    pub fn write_hs2(&mut self, payload: &[u8]) -> Result<Object, JsError> {
        let state = self.inner.take().ok_or(err(Error::BadState))?;
        let (next, hs2) = state.write_hs2(payload, &mut rng()).map_err(err)?;
        let o = Object::new();
        set(&o, "state", LinkCreator { inner: Some(next) });
        set(&o, "hs2", bytes(&hs2));
        Ok(o)
    }
}

#[wasm_bindgen]
pub struct LinkCreator {
    inner: Option<hs::LinkCreator>,
}

#[wasm_bindgen]
impl LinkCreator {
    /// `{ payload, session, sas, hash }`.
    #[wasm_bindgen(js_name = readHs3)]
    pub fn read_hs3(&mut self, hs3: &[u8], now: f64) -> Result<Object, JsError> {
        let state = self.inner.take().ok_or(err(Error::BadState))?;
        let (payload, est) = state.read_hs3(hs3).map_err(err)?;
        let o = established(&est, now);
        set(&o, "payload", bytes(&payload));
        Ok(o)
    }
}

fn established(est: &Established, now: f64) -> Object {
    let o = Object::new();
    set(
        &o,
        "session",
        Session {
            inner: session::Session::new(est, now_ms(now)),
        },
    );
    set(&o, "hash", bytes(&est.hash));
    match est.sas {
        Some(s) => set(&o, "sas", bytes(&s)),
        None => set(&o, "sas", JsValue::NULL),
    }
    o
}

// --------------------------------------------------------------------------------------------
// Signaling channels

/// Seals and opens envelopes on a family of daily topics, dropping stale and replayed ones.
#[wasm_bindgen]
pub struct Channel {
    key: [u8; 32],
    topic: Box<dyn Fn(u32) -> [u8; 32]>,
    guard: ReplayGuard,
}

impl Channel {
    fn new(key: [u8; 32], topic: Box<dyn Fn(u32) -> [u8; 32]>) -> Self {
        Self {
            key,
            topic,
            guard: ReplayGuard::default(),
        }
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.key = [0u8; 32];
    }
}

#[wasm_bindgen]
impl Channel {
    /// Rendezvous for a saved contact.
    #[wasm_bindgen(js_name = forContact)]
    pub fn for_contact(secret: &[u8]) -> Result<Channel, JsError> {
        let keys = contact::ContactKeys::new(&arr32(secret)?);
        let key = keys.env_key();
        Ok(Self::new(key, Box::new(move |d| keys.topic(d))))
    }

    /// Rendezvous for an identifier (lookups and contact requests).
    #[wasm_bindgen(js_name = forId)]
    pub fn for_id(id: &str) -> Result<Channel, JsError> {
        let keys = lookup::LookupKeys::new(&id::decode(id).map_err(err)?);
        let key = keys.env_key();
        Ok(Self::new(key, Box::new(move |d| keys.topic(d))))
    }

    pub fn topic(&self, day: u32) -> Vec<u8> {
        (self.topic)(day).to_vec()
    }

    /// Seals `body` on `topic`. `to` is a 16-byte peer tag, or empty for everyone.
    pub fn seal(
        &self,
        topic: &[u8],
        from: &[u8],
        to: &[u8],
        kind: u8,
        body: &[u8],
        now: f64,
    ) -> Result<Vec<u8>, JsError> {
        let env = Envelope {
            ts_ms: now_ms(now),
            from: tag(from)?,
            to: if to.is_empty() { envelope::BROADCAST } else { tag(to)? },
            kind,
            body: body.to_vec(),
        };
        Ok(envelope::seal(&self.key, &arr32(topic)?, &env, &mut rng()))
    }

    /// `{ ts, from, to (null for broadcast), kind, body }`; throws for anything not for us, stale or replayed.
    pub fn open(&mut self, topic: &[u8], data: &[u8], now: f64, manual: bool) -> Result<Object, JsError> {
        let env = envelope::open(&self.key, &arr32(topic)?, data).map_err(err)?;
        let max_age = if manual {
            envelope::MANUAL_MAX_AGE_MS
        } else {
            envelope::MAX_AGE_MS
        };
        self.guard.check(data, &env, now_ms(now), max_age).map_err(err)?;
        let o = Object::new();
        set(&o, "ts", env.ts_ms as f64);
        set(&o, "from", bytes(&env.from));
        if env.to == envelope::BROADCAST {
            set(&o, "to", JsValue::NULL);
        } else {
            set(&o, "to", bytes(&env.to));
        }
        set(&o, "kind", env.kind);
        set(&o, "body", bytes(&env.body));
        Ok(o)
    }
}

// --------------------------------------------------------------------------------------------
// Sessions

#[wasm_bindgen]
pub struct Session {
    inner: session::Session,
}

fn frames(list: Vec<(u8, Vec<u8>)>, field: &str) -> Array {
    let out = Array::new();
    for (lane, data) in list {
        let o = Object::new();
        set(&o, "lane", lane);
        set(&o, field, bytes(&data));
        out.push(&o);
    }
    out
}

#[wasm_bindgen]
impl Session {
    pub fn seal(&mut self, lane: u8, body: &[u8]) -> Result<Vec<u8>, JsError> {
        self.inner.seal(lane, body).map_err(err)
    }

    /// `{ messages: [{lane, data}], replies: [{lane, frame}], closed, rekeyed }`.
    pub fn open(&mut self, frame: &[u8], now: f64) -> Result<Object, JsError> {
        let opened = self.inner.open(frame, now_ms(now), &mut rng()).map_err(err)?;
        let o = Object::new();
        set(&o, "messages", frames(opened.messages, "data"));
        set(&o, "replies", frames(opened.replies, "frame"));
        set(&o, "closed", opened.closed);
        set(&o, "rekeyed", opened.rekeyed);
        Ok(o)
    }

    /// Frames to send now (scheduled rekeys): `[{lane, frame}]`.
    pub fn poll(&mut self, now: f64) -> Array {
        frames(self.inner.poll(now_ms(now), &mut rng()), "frame")
    }

    /// The farewell frame; erases every key.
    pub fn close(&mut self) -> Option<Vec<u8>> {
        self.inner.close()
    }

    #[wasm_bindgen(getter)]
    pub fn closed(&self) -> bool {
        self.inner.is_closed()
    }

    #[wasm_bindgen(getter)]
    pub fn epoch(&self) -> u32 {
        self.inner.epoch()
    }

    /// Throws `BadState` once the session is closed (its keys are erased).
    #[wasm_bindgen(js_name = contactSecret)]
    pub fn contact_secret(&self) -> Result<Vec<u8>, JsError> {
        if self.inner.is_closed() {
            return Err(err(Error::BadState));
        }
        Ok(self.inner.contact_secret().to_vec())
    }

    #[wasm_bindgen(js_name = handshakeHash)]
    pub fn handshake_hash(&self) -> Vec<u8> {
        self.inner.handshake_hash().to_vec()
    }

    /// Rendezvous of this session (relay fallback).
    pub fn channel(&self) -> Channel {
        let keys = self.inner.signal_keys();
        Channel::new(keys.env_key(), Box::new(move |d| keys.topic(d)))
    }
}

// --------------------------------------------------------------------------------------------
// Identities, bundles, cards

#[wasm_bindgen]
pub struct Identity {
    inner: CoreIdentity,
}

fn bundle_object(b: &bundle::Bundle) -> Object {
    let o = Object::new();
    set(&o, "id", b.identity.id());
    set(&o, "idHash", bytes(b.id_hash()));
    set(&o, "name", b.name.clone());
    set(&o, "spkEpoch", b.spk_epoch);
    set(&o, "spk", bytes(&b.spk));
    set(&o, "expiresMs", b.expires_ms as f64);
    set(&o, "bytes", bytes(&b.bytes));
    o
}

#[wasm_bindgen]
impl Identity {
    pub fn generate() -> Identity {
        Self {
            inner: CoreIdentity::generate(&mut rng()),
        }
    }

    #[wasm_bindgen(js_name = fromSeed)]
    pub fn from_seed(seed: &[u8]) -> Result<Identity, JsError> {
        Ok(Self {
            inner: CoreIdentity::from_seed(arr32(seed)?),
        })
    }

    pub fn seed(&self) -> Vec<u8> {
        self.inner.seed().to_vec()
    }

    #[wasm_bindgen(getter)]
    pub fn id(&self) -> String {
        self.inner.public().id()
    }

    #[wasm_bindgen(getter, js_name = idHash)]
    pub fn id_hash(&self) -> Vec<u8> {
        self.inner.public().id_hash().to_vec()
    }

    /// Our signed bundle (prekey for the current week).
    pub fn bundle(&self, name: &str, now: f64) -> Vec<u8> {
        bundle::Bundle::create(&self.inner, name, now_ms(now), &mut rng()).bytes
    }

    /// Our contact card for a session.
    pub fn card(&self, name: &str, session_hash: &[u8], now: f64) -> Result<Vec<u8>, JsError> {
        Ok(bundle::card(
            &self.inner,
            name,
            &arr32(session_hash)?,
            now_ms(now),
            &mut rng(),
        ))
    }

    /// Starts a contact handshake (`C1`) to a peer whose bundle we hold: `{ state, c1 }`.
    /// `psk` is the contact secret (the handshake key is derived from it), or empty for a first
    /// request (the lookup key is used).
    #[wasm_bindgen(js_name = contactStart)]
    pub fn contact_start(&self, psk: &[u8], peer_bundle: &[u8], payload: &[u8]) -> Result<Object, JsError> {
        let peer = bundle::Bundle::parse_stored(peer_bundle).map_err(err)?;
        let psk = contact_psk(psk, peer.id_hash())?;
        let params = ContactParams {
            psk: &psk,
            ids: (self.inner.public().id_hash(), peer.id_hash()),
        };
        let (state, c1) =
            hs::ContactInitiator::start(&params, &peer.spk, peer.spk_epoch, payload, &mut rng()).map_err(err)?;
        let o = Object::new();
        set(&o, "state", ContactInitiator { inner: Some(state) });
        set(&o, "c1", bytes(&c1));
        Ok(o)
    }

    /// Reads a `C1` sent to us by `peer_id_hash`: `{ state, payload }`. For a first request
    /// (`psk` empty) the peer is not known yet: pass its claimed id hash from the request.
    #[wasm_bindgen(js_name = contactRead)]
    pub fn contact_read(&self, psk: &[u8], peer_id_hash: &[u8], c1: &[u8], now: f64) -> Result<Object, JsError> {
        let me = self.inner.public().id_hash();
        let psk = contact_psk(psk, me)?;
        let peer = arr32(peer_id_hash)?;
        let params = ContactParams {
            psk: &psk,
            ids: (me, &peer),
        };
        let (state, payload) =
            hs::ContactResponderPending::read_c1(&params, &self.inner, epoch_window(now_ms(now)), c1).map_err(err)?;
        let o = Object::new();
        set(&o, "state", ContactResponderPending { inner: Some(state) });
        set(&o, "payload", bytes(&payload));
        Ok(o)
    }
}

/// The contact secret if known, else the lookup key of the responder's identifier.
/// The handshake key: from the contact secret (`PROTOCOL.md` §6: `Expand(croot, "…/contact/psk")`,
/// never the raw secret, which also derives the rendezvous), or the lookup key of the
/// responder's identifier for a first request.
fn contact_psk(secret: &[u8], responder: &[u8; 32]) -> Result<[u8; 32], JsError> {
    if secret.is_empty() {
        Ok(lookup::LookupKeys::new(responder).psk())
    } else {
        Ok(contact::ContactKeys::new(&arr32(secret)?).psk())
    }
}

#[wasm_bindgen]
pub struct ContactInitiator {
    inner: Option<hs::ContactInitiator>,
}

#[wasm_bindgen]
impl ContactInitiator {
    /// Reads C2: `{ state: ContactInitiatorReady, payload }` (payload = the responder's SDP offer;
    /// the answer goes back in C3 through `writeC3`).
    #[wasm_bindgen(js_name = readC2)]
    pub fn read_c2(&mut self, me: &Identity, c2: &[u8], now: f64) -> Result<Object, JsError> {
        let state = self.inner.take().ok_or(err(Error::BadState))?;
        let (ready, payload) = state.read_c2(&me.inner, epoch_window(now_ms(now)), c2).map_err(err)?;
        let o = Object::new();
        set(&o, "state", ContactInitiatorReady { inner: Some(ready) });
        set(&o, "payload", bytes(&payload));
        Ok(o)
    }
}

#[wasm_bindgen]
pub struct ContactInitiatorReady {
    inner: Option<hs::ContactInitiatorReady>,
}

#[wasm_bindgen]
impl ContactInitiatorReady {
    /// `{ c3, session, hash }`.
    #[wasm_bindgen(js_name = writeC3)]
    pub fn write_c3(&mut self, payload: &[u8], now: f64) -> Result<Object, JsError> {
        let state = self.inner.take().ok_or(err(Error::BadState))?;
        let (c3, est) = state.write_c3(payload);
        let o = established(&est, now);
        set(&o, "c3", bytes(&c3));
        Ok(o)
    }
}

#[wasm_bindgen]
pub struct ContactResponderPending {
    inner: Option<hs::ContactResponderPending>,
}

#[wasm_bindgen]
impl ContactResponderPending {
    /// `{ state, c2 }`, encapsulating to the initiator's prekey from its bundle.
    #[wasm_bindgen(js_name = writeC2)]
    pub fn write_c2(&mut self, peer_bundle: &[u8], payload: &[u8]) -> Result<Object, JsError> {
        let peer = bundle::Bundle::parse_stored(peer_bundle).map_err(err)?;
        let state = self.inner.take().ok_or(err(Error::BadState))?;
        let (next, c2) = state
            .write_c2(&peer.spk, peer.spk_epoch, payload, &mut rng())
            .map_err(err)?;
        let o = Object::new();
        set(&o, "state", ContactResponder { inner: Some(next) });
        set(&o, "c2", bytes(&c2));
        Ok(o)
    }
}

#[wasm_bindgen]
pub struct ContactResponder {
    inner: Option<hs::ContactResponder>,
}

#[wasm_bindgen]
impl ContactResponder {
    /// `{ payload, session, hash }`.
    #[wasm_bindgen(js_name = readC3)]
    pub fn read_c3(&mut self, c3: &[u8], now: f64) -> Result<Object, JsError> {
        let state = self.inner.take().ok_or(err(Error::BadState))?;
        let (payload, est) = state.read_c3(c3).map_err(err)?;
        let o = established(&est, now);
        set(&o, "payload", bytes(&payload));
        Ok(o)
    }
}

/// Verifies a peer's card for a session: `{ id, idHash, name, spkEpoch, spk, expiresMs, bytes }`
/// where `bytes` is the bundle to store.
#[wasm_bindgen(js_name = verifyCard)]
pub fn verify_card(card: &[u8], session_hash: &[u8], now: f64) -> Result<Object, JsError> {
    let b = bundle::verify_card(card, &arr32(session_hash)?, now_ms(now)).map_err(err)?;
    Ok(bundle_object(&b))
}

/// Verifies a stored or received bundle.
#[wasm_bindgen(js_name = parseBundle)]
pub fn parse_bundle(data: &[u8], now: f64) -> Result<Object, JsError> {
    let b = bundle::Bundle::parse(data, now_ms(now)).map_err(err)?;
    Ok(bundle_object(&b))
}

/// Verifies a bundle we stored ourselves (a saved contact's), ignoring its expiry.
#[wasm_bindgen(js_name = parseStoredBundle)]
pub fn parse_stored_bundle(data: &[u8]) -> Result<Object, JsError> {
    let b = bundle::Bundle::parse_stored(data).map_err(err)?;
    Ok(bundle_object(&b))
}

/// The identifier hash of a `hush1…` string (throws on typos).
#[wasm_bindgen(js_name = idDecode)]
pub fn id_decode(s: &str) -> Result<Vec<u8>, JsError> {
    Ok(id::decode(s).map_err(err)?.to_vec())
}

#[wasm_bindgen(js_name = idEncode)]
pub fn id_encode(hash: &[u8]) -> Result<String, JsError> {
    Ok(id::encode(&arr32(hash)?))
}

// --------------------------------------------------------------------------------------------
// Proof of work for unsolicited requests

#[wasm_bindgen(js_name = powChallenge)]
pub fn pow_challenge(id: &str, day: u32, c1: &[u8]) -> Result<Vec<u8>, JsError> {
    Ok(lookup::LookupKeys::new(&id::decode(id).map_err(err)?).challenge(day, c1))
}

#[wasm_bindgen(js_name = powSolve)]
pub fn pow_solve(challenge: &[u8], bits: u32) -> f64 {
    pow::solve(challenge, bits) as f64
}

#[wasm_bindgen(js_name = powCheck)]
pub fn pow_check(challenge: &[u8], nonce: f64, bits: u32) -> bool {
    nonce.is_finite() && nonce >= 0.0 && pow::check(challenge, nonce as u64, bits)
}

#[wasm_bindgen(js_name = powBits)]
pub fn pow_bits() -> u32 {
    pow::DEFAULT_BITS
}

// --------------------------------------------------------------------------------------------
// Vault and backups

#[wasm_bindgen]
pub struct Vault {
    inner: vault::Vault,
}

#[wasm_bindgen]
impl Vault {
    /// `{ vault, header }`.
    pub fn create(passphrase: &str) -> Result<Object, JsError> {
        let (v, header) = vault::Vault::create(passphrase, KdfParams::DEFAULT, &mut rng()).map_err(err)?;
        let o = Object::new();
        set(&o, "vault", Vault { inner: v });
        set(&o, "header", bytes(&header));
        Ok(o)
    }

    pub fn unlock(passphrase: &str, header: &[u8]) -> Result<Vault, JsError> {
        Ok(Self {
            inner: vault::Vault::unlock(passphrase, header).map_err(err)?,
        })
    }

    /// A new header for a new passphrase (records stay valid).
    pub fn rewrap(&self, passphrase: &str) -> Result<Vec<u8>, JsError> {
        self.inner
            .header(passphrase, KdfParams::DEFAULT, &mut rng())
            .map_err(err)
    }

    pub fn seal(&self, kind: &str, id: &str, plaintext: &[u8]) -> Vec<u8> {
        self.inner.seal(kind, id, plaintext, &mut rng())
    }

    pub fn open(&self, kind: &str, id: &str, blob: &[u8]) -> Result<Vec<u8>, JsError> {
        self.inner.open(kind, id, blob).map_err(err)
    }

    pub fn index(&self, kind: &str, id: &str) -> String {
        self.inner.index(kind, id)
    }
}

#[wasm_bindgen(js_name = backupExport)]
pub fn backup_export(passphrase: &str, snapshot: &[u8]) -> Result<Vec<u8>, JsError> {
    backup::export(passphrase, KdfParams::DEFAULT, snapshot, &mut rng()).map_err(err)
}

#[wasm_bindgen(js_name = backupImport)]
pub fn backup_import(passphrase: &str, file: &[u8]) -> Result<Vec<u8>, JsError> {
    backup::import(passphrase, file).map_err(err)
}

#[wasm_bindgen(js_name = isBackup)]
pub fn is_backup(file: &[u8]) -> bool {
    backup::is_backup(file)
}

// --------------------------------------------------------------------------------------------
// SAS

/// `[[emoji, name], …]` for SAS indices.
#[wasm_bindgen(js_name = sasEmojis)]
pub fn sas_emojis(indices: &[u8]) -> Array {
    let out = Array::new();
    for &i in indices {
        let (e, n) = sas::EMOJIS[usize::from(i & 63)];
        let pair = Array::new();
        pair.push(&JsValue::from_str(e));
        pair.push(&JsValue::from_str(n));
        out.push(&pair);
    }
    out
}
