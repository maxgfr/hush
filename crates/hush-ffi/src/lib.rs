//! UniFFI bindings for the Android (Kotlin) and iOS (Swift) apps.
//!
//! - `HushApp` (in `app.rs`) is the whole engine of `hush-app`: conversations, contacts, the
//!   vault, every network transport, and the Bluetooth and Wi-Fi links the app's radios open.
//! - The sans-IO pieces of `hush-core` stay exported below (links, handshakes, sessions,
//!   framing), for tests and tools.
//!
//! Generate the bindings with:
//!
//! ```sh
//! cargo build -p hush-ffi --profile ffi
//! cargo run -p hush-ffi --features cli --bin uniffi-bindgen -- generate \
//!   --library target/ffi/libhush_ffi.dylib --language kotlin --out-dir bindings/kotlin
//! ```
#![forbid(unsafe_code)]

mod app;
pub use app::*;

use std::sync::{Arc, Mutex};

use getrandom::SysRng;
use hush_core::envelope::{self, Envelope as CoreEnvelope, ReplayGuard};
use hush_core::handshake::{self as hs, Established};
use hush_core::link::{Link, LinkKeys, Mode};
use hush_core::{backup, bundle, sas, session, stream};
use rand_core::UnwrapErr;

uniffi::setup_scaffolding!();

fn rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

/// Every failure, named like the core error (`Decrypt`, `Expired`, …).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum HushError {
    #[error("{name}")]
    Core { name: String },
    /// From the engine: a message to show, or a core error name ("Decrypt" for a wrong passphrase).
    #[error("{message}")]
    App { message: String },
}

impl From<hush_core::Error> for HushError {
    fn from(e: hush_core::Error) -> Self {
        Self::Core { name: format!("{e:?}") }
    }
}

type Res<T> = Result<T, HushError>;

fn bad() -> HushError {
    hush_core::Error::Malformed.into()
}

fn arr<const N: usize>(b: &[u8]) -> Res<[u8; N]> {
    b.try_into().map_err(|_| bad())
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Days since the Unix epoch, the unit of rendezvous topics.
#[uniffi::export]
pub fn utc_day(now_ms: u64) -> u32 {
    hush_core::utc_day(now_ms)
}

/// A new link under `base` (e.g. `https://maxgfr.github.io/hush/`).
#[uniffi::export]
pub fn link_new(group: bool, ttl_min: u64, passphrase: bool, now_ms: u64, base: String) -> String {
    let mode = if group { Mode::Group } else { Mode::Direct };
    Link::generate(mode, now_ms, ttl_min, passphrase, &mut rng()).url(&base)
}

/// Human-readable emojis for SAS indices.
#[uniffi::export]
pub fn sas_render(indices: Vec<u8>) -> String {
    sas::render(&indices)
}

/// The spoken names of SAS indices (English).
#[uniffi::export]
pub fn sas_names(indices: Vec<u8>) -> String {
    sas::names(&indices)
}

// ── signaling envelopes ─────────────────────────────────────────────────────

#[derive(uniffi::Record)]
pub struct Envelope {
    pub ts_ms: u64,
    pub from: Vec<u8>,
    /// None when addressed to everyone on the topic.
    pub to: Option<Vec<u8>>,
    pub kind: u8,
    pub body: Vec<u8>,
}

/// Seals and opens envelopes on a family of daily topics.
#[derive(uniffi::Object)]
pub struct Channel {
    key: [u8; 32],
    topic: Box<dyn Fn(u32) -> [u8; 32] + Send + Sync>,
    guard: Mutex<ReplayGuard>,
}

#[uniffi::export]
impl Channel {
    pub fn topic(&self, day: u32) -> Vec<u8> {
        (self.topic)(day).to_vec()
    }

    pub fn seal(
        &self,
        topic: Vec<u8>,
        from: Vec<u8>,
        to: Option<Vec<u8>>,
        kind: u8,
        body: Vec<u8>,
        now_ms: u64,
    ) -> Res<Vec<u8>> {
        let env = CoreEnvelope {
            ts_ms: now_ms,
            from: arr(&from)?,
            to: match to {
                Some(t) => arr(&t)?,
                None => envelope::BROADCAST,
            },
            kind,
            body,
        };
        Ok(envelope::seal(&self.key, &arr(&topic)?, &env, &mut rng()))
    }

    /// Opens an envelope; fails for anything stale, replayed or not sealed for this channel.
    pub fn open(&self, topic: Vec<u8>, data: Vec<u8>, now_ms: u64, manual: bool) -> Res<Envelope> {
        let env = envelope::open(&self.key, &arr(&topic)?, &data)?;
        let max_age = if manual {
            envelope::MANUAL_MAX_AGE_MS
        } else {
            envelope::MAX_AGE_MS
        };
        locked(&self.guard).check(&data, &env, now_ms, max_age)?;
        Ok(Envelope {
            ts_ms: env.ts_ms,
            from: env.from.to_vec(),
            to: (env.to != envelope::BROADCAST).then(|| env.to.to_vec()),
            kind: env.kind,
            body: env.body,
        })
    }
}

fn channel(key: [u8; 32], topic: impl Fn(u32) -> [u8; 32] + Send + Sync + 'static) -> Arc<Channel> {
    Arc::new(Channel {
        key,
        topic: Box::new(topic),
        guard: Mutex::new(ReplayGuard::default()),
    })
}

// ── link handshake ──────────────────────────────────────────────────────────

#[derive(uniffi::Object)]
pub struct LinkSecrets {
    keys: LinkKeys,
}

#[derive(uniffi::Record)]
pub struct JoinerStart {
    pub state: Arc<LinkJoiner>,
    pub hs1: Vec<u8>,
}

#[derive(uniffi::Record)]
pub struct CreatorRead {
    pub state: Arc<LinkCreatorPending>,
    pub payload: Vec<u8>,
}

#[uniffi::export]
impl LinkSecrets {
    /// Derives the keys of a link (slow with a passphrase: Argon2id, 64 MiB).
    #[uniffi::constructor]
    pub fn new(input: String, passphrase: Option<String>) -> Res<Arc<Self>> {
        let link = Link::parse(&input)?;
        Ok(Arc::new(Self {
            keys: link.keys(passphrase.as_deref())?,
        }))
    }

    pub fn group(&self) -> bool {
        self.keys.mode == Mode::Group
    }

    pub fn expires_ms(&self) -> u64 {
        self.keys.expires_min * 60_000
    }

    pub fn channel(&self) -> Arc<Channel> {
        let keys = self.keys.clone();
        channel(*self.keys.env_key, move |d| keys.topic(d))
    }

    pub fn joiner_start(&self, payload: Vec<u8>) -> JoinerStart {
        let (state, hs1) = hs::LinkJoiner::start(&self.keys, &payload, &mut rng());
        JoinerStart {
            state: Arc::new(LinkJoiner { inner: state }),
            hs1,
        }
    }

    pub fn creator_read_hs1(&self, hs1: Vec<u8>) -> Res<CreatorRead> {
        let (state, payload) = hs::LinkCreatorPending::read_hs1(&self.keys, &hs1)?;
        Ok(CreatorRead {
            state: Arc::new(LinkCreatorPending {
                inner: Mutex::new(Some(state)),
            }),
            payload,
        })
    }
}

#[derive(uniffi::Object)]
pub struct LinkJoiner {
    inner: hs::LinkJoiner,
}

#[derive(uniffi::Record)]
pub struct JoinerRead {
    pub state: Arc<LinkJoinerReady>,
    pub payload: Vec<u8>,
}

#[uniffi::export]
impl LinkJoiner {
    /// Reads one member's HS2 (callable once per member in a group).
    pub fn read_hs2(&self, hs2: Vec<u8>) -> Res<JoinerRead> {
        let (state, payload) = self.inner.read_hs2(&hs2)?;
        Ok(JoinerRead {
            state: Arc::new(LinkJoinerReady {
                inner: Mutex::new(Some(state)),
            }),
            payload,
        })
    }
}

/// A finished handshake: the last message to send (or the peer's last payload) and the session.
#[derive(uniffi::Record)]
pub struct Finished {
    pub message: Vec<u8>,
    pub session: Arc<Session>,
    pub sas: Vec<u8>,
    pub hash: Vec<u8>,
}

fn finished(message: Vec<u8>, est: &Established, now_ms: u64) -> Finished {
    Finished {
        message,
        session: Arc::new(Session {
            inner: Mutex::new(session::Session::new(est, now_ms)),
        }),
        sas: est.sas.map(|s| s.to_vec()).unwrap_or_default(),
        hash: est.hash.to_vec(),
    }
}

#[derive(uniffi::Object)]
pub struct LinkJoinerReady {
    inner: Mutex<Option<hs::LinkJoinerReady>>,
}

#[uniffi::export]
impl LinkJoinerReady {
    /// `message` is HS3, to send.
    pub fn write_hs3(&self, payload: Vec<u8>, now_ms: u64) -> Res<Finished> {
        let state = locked(&self.inner)
            .take()
            .ok_or(HushError::from(hush_core::Error::BadState))?;
        let (hs3, est) = state.write_hs3(&payload);
        Ok(finished(hs3, &est, now_ms))
    }
}

#[derive(uniffi::Object)]
pub struct LinkCreatorPending {
    inner: Mutex<Option<hs::LinkCreatorPending>>,
}

#[derive(uniffi::Record)]
pub struct CreatorWrote {
    pub state: Arc<LinkCreator>,
    pub hs2: Vec<u8>,
}

#[uniffi::export]
impl LinkCreatorPending {
    pub fn write_hs2(&self, payload: Vec<u8>) -> Res<CreatorWrote> {
        let state = locked(&self.inner)
            .take()
            .ok_or(HushError::from(hush_core::Error::BadState))?;
        let (next, hs2) = state.write_hs2(&payload, &mut rng())?;
        Ok(CreatorWrote {
            state: Arc::new(LinkCreator {
                inner: Mutex::new(Some(next)),
            }),
            hs2,
        })
    }
}

#[derive(uniffi::Object)]
pub struct LinkCreator {
    inner: Mutex<Option<hs::LinkCreator>>,
}

#[uniffi::export]
impl LinkCreator {
    /// `message` is the joiner's last payload.
    pub fn read_hs3(&self, hs3: Vec<u8>, now_ms: u64) -> Res<Finished> {
        let state = locked(&self.inner)
            .take()
            .ok_or(HushError::from(hush_core::Error::BadState))?;
        let (payload, est) = state.read_hs3(&hs3)?;
        Ok(finished(payload, &est, now_ms))
    }
}

// ── sessions ────────────────────────────────────────────────────────────────

#[derive(uniffi::Record)]
pub struct LaneData {
    pub lane: u8,
    pub data: Vec<u8>,
}

#[derive(uniffi::Record)]
pub struct Opened {
    pub messages: Vec<LaneData>,
    pub replies: Vec<LaneData>,
    pub closed: bool,
    pub rekeyed: bool,
}

fn lanes(v: Vec<(u8, Vec<u8>)>) -> Vec<LaneData> {
    v.into_iter().map(|(lane, data)| LaneData { lane, data }).collect()
}

#[derive(uniffi::Object)]
pub struct Session {
    inner: Mutex<session::Session>,
}

#[uniffi::export]
impl Session {
    pub fn seal(&self, lane: u8, body: Vec<u8>) -> Res<Vec<u8>> {
        Ok(locked(&self.inner).seal(lane, &body)?)
    }

    pub fn open(&self, frame: Vec<u8>, now_ms: u64) -> Res<Opened> {
        let o = locked(&self.inner).open(&frame, now_ms, &mut rng())?;
        Ok(Opened {
            messages: lanes(o.messages),
            replies: lanes(o.replies),
            closed: o.closed,
            rekeyed: o.rekeyed,
        })
    }

    /// Frames due now (scheduled post-quantum rekeys).
    pub fn poll(&self, now_ms: u64) -> Vec<LaneData> {
        lanes(locked(&self.inner).poll(now_ms, &mut rng()))
    }

    /// The farewell frame; erases every key.
    pub fn close(&self) -> Option<Vec<u8>> {
        locked(&self.inner).close()
    }

    pub fn contact_secret(&self) -> Vec<u8> {
        locked(&self.inner).contact_secret().to_vec()
    }

    pub fn handshake_hash(&self) -> Vec<u8> {
        locked(&self.inner).handshake_hash().to_vec()
    }

    /// Rendezvous of this session (relay fallback).
    pub fn channel(&self) -> Arc<Channel> {
        let keys = locked(&self.inner).signal_keys();
        channel(keys.env_key(), move |d| keys.topic(d))
    }
}

// ── identities and backups ──────────────────────────────────────────────────

#[derive(uniffi::Object)]
pub struct Identity {
    inner: hush_core::identity::Identity,
}

#[derive(uniffi::Record)]
pub struct BundleInfo {
    pub id: String,
    pub name: String,
    pub expires_ms: u64,
    pub bytes: Vec<u8>,
}

#[uniffi::export]
impl Identity {
    #[uniffi::constructor]
    pub fn generate() -> Arc<Self> {
        Arc::new(Self {
            inner: hush_core::identity::Identity::generate(&mut rng()),
        })
    }

    #[uniffi::constructor]
    pub fn from_seed(seed: Vec<u8>) -> Res<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: hush_core::identity::Identity::from_seed(arr(&seed)?),
        }))
    }

    pub fn seed(&self) -> Vec<u8> {
        self.inner.seed().to_vec()
    }

    /// The self-certifying `hush1…` identifier.
    pub fn id(&self) -> String {
        self.inner.public().id()
    }

    pub fn card(&self, name: String, session_hash: Vec<u8>, now_ms: u64) -> Res<Vec<u8>> {
        Ok(bundle::card(
            &self.inner,
            &name,
            &arr(&session_hash)?,
            now_ms,
            &mut rng(),
        ))
    }
}

/// Verifies a peer's contact card for a session.
#[uniffi::export]
pub fn verify_card(card: Vec<u8>, session_hash: Vec<u8>, now_ms: u64) -> Res<BundleInfo> {
    let b = bundle::verify_card(&card, &arr(&session_hash)?, now_ms)?;
    Ok(BundleInfo {
        id: b.identity.id(),
        name: b.name,
        expires_ms: b.expires_ms,
        bytes: b.bytes,
    })
}

#[uniffi::export]
pub fn backup_export(passphrase: String, snapshot: Vec<u8>) -> Res<Vec<u8>> {
    Ok(backup::export(
        &passphrase,
        hush_core::vault::KdfParams::DEFAULT,
        &snapshot,
        &mut rng(),
    )?)
}

#[uniffi::export]
pub fn backup_import(passphrase: String, file: Vec<u8>) -> Res<Vec<u8>> {
    Ok(backup::import(&passphrase, &file)?)
}

// ── framing for Bluetooth and Wi-Fi links ───────────────────────────────────

/// One record for a byte stream (TCP over Wi-Fi, BLE L2CAP).
#[uniffi::export]
pub fn encode_record(lane: u8, frame: Vec<u8>) -> Vec<u8> {
    stream::encode_record(lane, &frame)
}

#[derive(uniffi::Object)]
pub struct StreamDecoder {
    inner: Mutex<stream::StreamDecoder>,
}

#[uniffi::export]
impl StreamDecoder {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(stream::StreamDecoder::new()),
        })
    }

    pub fn push(&self, data: Vec<u8>) -> Res<Vec<LaneData>> {
        Ok(lanes(locked(&self.inner).push(&data)?))
    }
}

/// Splits records into packets for small-MTU links (BLE GATT).
#[derive(uniffi::Object)]
pub struct Fragmenter {
    inner: Mutex<stream::Fragmenter>,
}

#[uniffi::export]
impl Fragmenter {
    #[uniffi::constructor]
    pub fn new(mtu: u32) -> Res<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: Mutex::new(stream::Fragmenter::new(mtu as usize)?),
        }))
    }

    pub fn split(&self, lane: u8, frame: Vec<u8>) -> Vec<Vec<u8>> {
        locked(&self.inner).split(lane, &frame)
    }
}

#[derive(uniffi::Object)]
pub struct Reassembler {
    inner: Mutex<stream::Reassembler>,
}

#[uniffi::export]
impl Reassembler {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(stream::Reassembler::new()),
        })
    }

    /// A complete record once its last packet arrives.
    pub fn push(&self, packet: Vec<u8>) -> Res<Option<LaneData>> {
        Ok(locked(&self.inner)
            .push(&packet)?
            .map(|(lane, data)| LaneData { lane, data }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole mobile flow over a simulated Bluetooth link with a 185-byte MTU.
    #[test]
    fn handshake_and_session_over_ble_sized_packets() {
        let now = 1_750_000_000_000;
        let url = link_new(false, 60, false, now, "https://maxgfr.github.io/hush/".into());
        let creator = LinkSecrets::new(url.clone(), None).unwrap();
        let joiner = LinkSecrets::new(url, None).unwrap();
        let mut ble = stream::Fragmenter::new(185).unwrap();
        let mut rx = stream::Reassembler::new();
        let mut carry = |lane: u8, bytes: Vec<u8>| -> Vec<u8> {
            let mut out = None;
            for p in ble.split(lane, &bytes) {
                if let Some(r) = rx.push(&p).unwrap() {
                    out = Some(r);
                }
            }
            out.unwrap().1
        };
        let start = joiner.joiner_start(b"hello".to_vec());
        let read = creator.creator_read_hs1(carry(stream::SIGNAL_LANE, start.hs1)).unwrap();
        assert_eq!(read.payload, b"hello");
        let wrote = read.state.write_hs2(b"offer".to_vec()).unwrap();
        let jr = start.state.read_hs2(carry(stream::SIGNAL_LANE, wrote.hs2)).unwrap();
        let j = jr.state.write_hs3(b"answer".to_vec(), now).unwrap();
        let c = wrote
            .state
            .read_hs3(carry(stream::SIGNAL_LANE, j.message.clone()), now)
            .unwrap();
        assert_eq!(c.message, b"answer");
        assert_eq!(j.sas, c.sas);
        let frame = j.session.seal(1, b"over bluetooth".to_vec()).unwrap();
        let opened = c.session.open(carry(1, frame), now).unwrap();
        assert_eq!(opened.messages[0].data, b"over bluetooth");
        assert!(matches!(
            j.session.open(vec![1, 2, 3], now),
            Err(HushError::Core { .. })
        ));
    }
}
