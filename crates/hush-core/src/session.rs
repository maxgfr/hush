//! The session ratchet, once a handshake is done.
//!
//! - **Forward secrecy per message**: each (direction, lane) has a KDF chain; every message key is
//!   used once and forgotten (`mk = HMAC(ck, 1)`, `ck = HMAC(ck, 2)`).
//! - **Post-compromise security, post-quantum**: every 5 minutes or 1000 messages the initiator
//!   sends a fresh X-Wing key; the responder encapsulates to it and both move to
//!   `root' = Extract(root, ss || epoch)`. Old epochs are erased.
//! - **Length hiding**: every plaintext is Padmé-padded.
//!
//! ```text
//! frame  = v(1) || lane(1) || epoch(u32) || ctr(u64) || ChaCha20-Poly1305(mk, nonce = ctr, aad = header || session_id, padded(type || body))
//! ```

use std::collections::BTreeMap;

use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::aead::{counter_nonce, open, seal};
use crate::handshake::Established;
use crate::kdf::{expand16, expand32, extract, hmac};
use crate::kem::{KemSecret, encapsulate};
use crate::{Error, Result, Rng, VERSION};

/// Logical lanes, each with its own chain so one cannot stall another.
pub mod lane {
    /// Protocol control (rekeys, bye) and application control messages.
    pub const CTRL: u8 = 0;
    /// Chat messages.
    pub const CHAT: u8 = 1;
    /// File transfer.
    pub const FILE: u8 = 2;
    /// Frames relayed over signaling (reconnection offers, relay fallback).
    pub const SIG: u8 = 3;
}
const LANES: usize = 4;

/// Largest plaintext a frame carries.
pub const MAX_PLAINTEXT: usize = 256 * 1024;
/// Largest counter jump accepted on one chain (for unordered transports).
pub const MAX_SKIP: u64 = 1000;
/// Rekey cadence.
pub const REKEY_INTERVAL_MS: u64 = 5 * 60_000;
pub const REKEY_MESSAGES: u64 = 1000;
/// How long the previous epoch's receive chains survive a rekey.
pub const PREVIOUS_EPOCH_GRACE_MS: u64 = 120_000;
/// Frames from the next epoch held until its key arrives.
const MAX_BUFFERED: usize = 256;

const HEADER_LEN: usize = 14;

mod inner {
    pub const APP: u8 = 0;
    pub const REKEY: u8 = 1;
    pub const REKEY_ACK: u8 = 2;
    pub const BYE: u8 = 3;
}

#[derive(Zeroize, ZeroizeOnDrop, Clone)]
struct Chain {
    ck: [u8; 32],
    n: u64,
}

impl Chain {
    fn new(root: &[u8; 32], dir: u8, lane: u8) -> Self {
        Self {
            ck: expand32(root, b"hush/v1/session/chain", &[&[dir, lane]]),
            n: 0,
        }
    }

    /// Returns the key for message `n` and advances.
    fn step(&mut self) -> ([u8; 32], u64) {
        let mk = hmac(&self.ck, &[&[1]]);
        self.ck = hmac(&self.ck, &[&[2]]);
        let n = self.n;
        self.n += 1;
        (mk, n)
    }
}

#[derive(Clone)]
struct RecvChain {
    chain: Chain,
    skipped: BTreeMap<u64, [u8; 32]>,
}

impl Drop for RecvChain {
    fn drop(&mut self) {
        for v in self.skipped.values_mut() {
            v.zeroize();
        }
    }
}

impl RecvChain {
    /// Finds the key for counter `ctr` without committing; returns the would-be new state.
    fn key_for(&self, ctr: u64) -> Result<([u8; 32], Option<RecvChain>)> {
        if ctr < self.chain.n {
            return self.skipped.get(&ctr).map(|k| (*k, None)).ok_or(Error::Replay);
        }
        if ctr - self.chain.n > MAX_SKIP {
            return Err(Error::TooFarAhead);
        }
        let mut next = self.clone();
        while next.chain.n < ctr {
            let (k, n) = next.chain.step();
            next.skipped.insert(n, k);
        }
        while next.skipped.len() > MAX_SKIP as usize {
            let first = *next.skipped.keys().next().expect("non-empty");
            if let Some(mut k) = next.skipped.remove(&first) {
                k.zeroize();
            }
        }
        let (k, _) = next.chain.step();
        Ok((k, Some(next)))
    }
}

struct Epoch {
    number: u32,
    root: Zeroizing<[u8; 32]>,
    send: Vec<Chain>,
    recv: Vec<RecvChain>,
    started_ms: u64,
    count: u64,
}

impl Epoch {
    fn new(number: u32, root: [u8; 32], initiator: bool, now_ms: u64) -> Self {
        let (send_dir, recv_dir) = if initiator { (0, 1) } else { (1, 0) };
        let lanes = 0..u8::try_from(LANES).expect("4 lanes");
        Self {
            number,
            send: lanes.clone().map(|l| Chain::new(&root, send_dir, l)).collect(),
            recv: lanes
                .map(|l| RecvChain {
                    chain: Chain::new(&root, recv_dir, l),
                    skipped: BTreeMap::new(),
                })
                .collect(),
            root: Zeroizing::new(root),
            started_ms: now_ms,
            count: 0,
        }
    }

    fn next_root(&self, shared: &[u8; 32]) -> [u8; 32] {
        let next = self.number + 1;
        let mut ikm = Zeroizing::new(shared.to_vec());
        ikm.extend_from_slice(b"hush/v1/session/rekey");
        ikm.extend_from_slice(&next.to_be_bytes());
        extract(&*self.root, &ikm)
    }
}

/// Rendezvous keys of an established session (reconnection offers, relay fallback).
#[derive(Clone)]
pub struct SignalKeys {
    root: Zeroizing<[u8; 32]>,
}

impl SignalKeys {
    #[must_use]
    pub fn env_key(&self) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/session/signal/env", &[])
    }

    #[must_use]
    pub fn topic(&self, day: u32) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/session/signal/topic", &[&day.to_be_bytes()])
    }
}

/// What opening a frame produced.
#[derive(Debug, Default)]
pub struct Opened {
    /// Application messages, in order: `(lane, bytes)`.
    pub messages: Vec<(u8, Vec<u8>)>,
    /// Frames to send back (rekey acknowledgements).
    pub replies: Vec<(u8, Vec<u8>)>,
    /// The peer ended the session; every key has been erased.
    pub closed: bool,
    /// The session moved to a new epoch.
    pub rekeyed: bool,
}

/// An established, ratcheting session.
pub struct Session {
    initiator: bool,
    session_id: [u8; 16],
    hash: [u8; 32],
    contact_base: Zeroizing<[u8; 32]>,
    signal_root: Zeroizing<[u8; 32]>,
    cur: Epoch,
    prev: Option<(Vec<RecvChain>, u32, u64)>,
    pending: Option<(u32, KemSecret, u64)>,
    buffered: Vec<Vec<u8>>,
    closed: bool,
}

impl core::fmt::Debug for Session {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Session")
            .field("epoch", &self.cur.number)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

fn header(lane: u8, epoch: u32, ctr: u64) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0] = VERSION;
    h[1] = lane;
    h[2..6].copy_from_slice(&epoch.to_be_bytes());
    h[6..].copy_from_slice(&ctr.to_be_bytes());
    h
}

impl Session {
    #[must_use]
    pub fn new(est: &Established, now_ms: u64) -> Self {
        let root = *est.root;
        Self {
            initiator: est.initiator,
            session_id: expand16(&root, b"hush/v1/session/id", &[]),
            hash: est.hash,
            contact_base: Zeroizing::new(expand32(&root, b"hush/v1/session/contact", &[])),
            signal_root: Zeroizing::new(expand32(&root, b"hush/v1/session/signal", &[])),
            cur: Epoch::new(
                0,
                expand32(&root, b"hush/v1/session/epoch0", &[]),
                est.initiator,
                now_ms,
            ),
            prev: None,
            pending: None,
            buffered: Vec::new(),
            closed: false,
        }
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.cur.number
    }

    /// The handshake transcript hash (contact cards sign over it).
    #[must_use]
    pub fn handshake_hash(&self) -> &[u8; 32] {
        &self.hash
    }

    /// Shared secret both sides store if they save each other as contacts.
    #[must_use]
    pub fn contact_secret(&self) -> [u8; 32] {
        *self.contact_base
    }

    /// Key and daily topic for signaling envelopes that belong to this session.
    #[must_use]
    pub fn signal_keys(&self) -> SignalKeys {
        SignalKeys {
            root: self.signal_root.clone(),
        }
    }

    fn seal_inner(&mut self, lane: u8, kind: u8, body: &[u8]) -> Result<Vec<u8>> {
        if self.closed {
            return Err(Error::BadState);
        }
        if usize::from(lane) >= LANES {
            return Err(Error::InvalidParameter);
        }
        if body.len() > MAX_PLAINTEXT {
            return Err(Error::InvalidParameter);
        }
        let (mk, ctr) = self.cur.send[usize::from(lane)].step();
        let mk = Zeroizing::new(mk);
        let h = header(lane, self.cur.number, ctr);
        let mut aad = h.to_vec();
        aad.extend_from_slice(&self.session_id);
        let mut pt = Zeroizing::new(Vec::with_capacity(body.len() + 1));
        pt.push(kind);
        pt.extend_from_slice(body);
        let padded = Zeroizing::new(crate::pad::pad(&pt));
        let mut frame = h.to_vec();
        frame.extend_from_slice(&seal(&mk, &counter_nonce(ctr), &aad, &padded));
        self.cur.count += 1;
        Ok(frame)
    }

    /// Encrypts an application message.
    ///
    /// # Errors
    /// [`Error::BadState`] once closed, [`Error::InvalidParameter`] for an unknown lane or oversize body.
    pub fn seal(&mut self, lane: u8, body: &[u8]) -> Result<Vec<u8>> {
        self.seal_inner(lane, inner::APP, body)
    }

    /// Timer hook: starts a rekey when due (initiator only) and expires the previous epoch.
    /// Returns frames to send.
    pub fn poll<R: Rng + ?Sized>(&mut self, now_ms: u64, rng: &mut R) -> Vec<(u8, Vec<u8>)> {
        if self.closed {
            return Vec::new();
        }
        if let Some((_, _, until)) = &self.prev
            && now_ms >= *until
        {
            self.prev = None;
        }
        if let Some((_, _, started)) = &self.pending {
            // An acknowledgement never came (the link dropped mid-rekey): allow a fresh attempt.
            if now_ms >= started + PREVIOUS_EPOCH_GRACE_MS {
                self.pending = None;
            }
        }
        let due = now_ms.saturating_sub(self.cur.started_ms) >= REKEY_INTERVAL_MS || self.cur.count >= REKEY_MESSAGES;
        if self.initiator && self.pending.is_none() && due {
            return self
                .start_rekey(now_ms, rng)
                .map(|f| vec![(lane::CTRL, f)])
                .unwrap_or_default();
        }
        Vec::new()
    }

    /// Forces a rekey now (initiator only; the responder may ask through the app).
    ///
    /// # Errors
    /// [`Error::BadState`] on the responder, when closed, or with a rekey already in flight.
    pub fn start_rekey<R: Rng + ?Sized>(&mut self, now_ms: u64, rng: &mut R) -> Result<Vec<u8>> {
        if !self.initiator || self.pending.is_some() {
            return Err(Error::BadState);
        }
        let next = self.cur.number + 1;
        let kem = KemSecret::generate(rng);
        let mut body = next.to_be_bytes().to_vec();
        body.extend_from_slice(&kem.public());
        let frame = self.seal_inner(lane::CTRL, inner::REKEY, &body)?;
        self.pending = Some((next, kem, now_ms));
        Ok(frame)
    }

    /// Ends the session: returns a final frame for the peer and erases every key.
    pub fn close(&mut self) -> Option<Vec<u8>> {
        let frame = self.seal_inner(lane::CTRL, inner::BYE, &[]).ok();
        self.wipe();
        frame
    }

    fn wipe(&mut self) {
        self.closed = true;
        self.prev = None;
        self.pending = None;
        self.buffered.clear();
        self.cur = Epoch::new(u32::MAX, [0u8; 32], self.initiator, 0);
        self.contact_base.zeroize();
        self.signal_root.zeroize();
    }

    /// Decrypts a frame and runs the protocol messages it carries.
    ///
    /// # Errors
    /// [`Error::Decrypt`] for forged or corrupt frames, [`Error::Replay`] for a frame already
    /// opened, [`Error::TooFarAhead`] for an absurd counter. The session state is unchanged on error.
    pub fn open<R: Rng + ?Sized>(&mut self, frame: &[u8], now_ms: u64, rng: &mut R) -> Result<Opened> {
        let mut out = Opened::default();
        self.open_into(frame, now_ms, rng, &mut out)?;
        Ok(out)
    }

    fn open_into<R: Rng + ?Sized>(&mut self, frame: &[u8], now_ms: u64, rng: &mut R, out: &mut Opened) -> Result<()> {
        if self.closed {
            return Err(Error::BadState);
        }
        if frame.len() < HEADER_LEN + crate::aead::TAG_LEN || frame[0] != VERSION {
            return Err(Error::Malformed);
        }
        let lane = frame[1];
        if usize::from(lane) >= LANES {
            return Err(Error::Malformed);
        }
        let epoch = u32::from_be_bytes(frame[2..6].try_into().map_err(|_| Error::Malformed)?);
        let ctr = u64::from_be_bytes(frame[6..14].try_into().map_err(|_| Error::Malformed)?);

        // A frame of the epoch we are about to enter: hold it until the acknowledgement arrives.
        if epoch == self.cur.number + 1 && self.pending.is_some() {
            if self.buffered.len() >= MAX_BUFFERED {
                return Err(Error::TooFarAhead);
            }
            self.buffered.push(frame.to_vec());
            return Ok(());
        }

        let chains = if epoch == self.cur.number {
            &self.cur.recv
        } else if let Some((prev, n, _)) = &self.prev
            && epoch == *n
        {
            prev
        } else {
            return Err(Error::Decrypt);
        };
        let (mk, next) = chains[usize::from(lane)].key_for(ctr)?;
        let mk = Zeroizing::new(mk);
        let mut aad = frame[..HEADER_LEN].to_vec();
        aad.extend_from_slice(&self.session_id);
        let padded = Zeroizing::new(open(&mk, &counter_nonce(ctr), &aad, &frame[HEADER_LEN..])?);
        let pt = crate::pad::unpad(&padded)?;
        let (&kind, body) = pt.split_first().ok_or(Error::Malformed)?;

        // Authentic: commit the chain state.
        let in_current = epoch == self.cur.number;
        let chains = if in_current {
            &mut self.cur.recv
        } else {
            &mut self.prev.as_mut().expect("checked").0
        };
        let slot = &mut chains[usize::from(lane)];
        match next {
            Some(n) => *slot = n,
            None => {
                if let Some(mut k) = slot.skipped.remove(&ctr) {
                    k.zeroize();
                }
            }
        }
        if in_current {
            self.cur.count += 1;
        }

        match kind {
            inner::APP => out.messages.push((lane, body.to_vec())),
            inner::BYE => {
                out.closed = true;
                self.wipe();
            }
            inner::REKEY if !self.initiator && lane == lane::CTRL && in_current => {
                let next = u32::from_be_bytes(
                    body.get(..4)
                        .ok_or(Error::Malformed)?
                        .try_into()
                        .map_err(|_| Error::Malformed)?,
                );
                if next != self.cur.number + 1 {
                    return Err(Error::Malformed);
                }
                let (ct, shared) = encapsulate(&body[4..], rng)?;
                let mut ack = next.to_be_bytes().to_vec();
                ack.extend_from_slice(&ct);
                // The acknowledgement still travels in the old epoch: the initiator cannot read the new one yet.
                let frame = self.seal_inner(lane::CTRL, inner::REKEY_ACK, &ack)?;
                out.replies.push((lane::CTRL, frame));
                self.advance(&shared, now_ms);
                out.rekeyed = true;
            }
            inner::REKEY_ACK if self.initiator && lane == lane::CTRL && in_current => {
                let next = u32::from_be_bytes(
                    body.get(..4)
                        .ok_or(Error::Malformed)?
                        .try_into()
                        .map_err(|_| Error::Malformed)?,
                );
                let Some((expected, kem, _)) = self.pending.take() else {
                    return Err(Error::BadState);
                };
                if next != expected {
                    return Err(Error::Malformed);
                }
                let shared = kem.decapsulate(&body[4..])?;
                self.advance(&shared, now_ms);
                out.rekeyed = true;
                for held in core::mem::take(&mut self.buffered) {
                    // Frames that fail here are dropped like any other forged frame.
                    let _ = self.open_into(&held, now_ms, rng, out);
                }
            }
            _ => return Err(Error::Malformed),
        }
        Ok(())
    }

    fn advance(&mut self, shared: &[u8; 32], now_ms: u64) {
        let root = self.cur.next_root(shared);
        let next = Epoch::new(self.cur.number + 1, root, self.initiator, now_ms);
        let old = core::mem::replace(&mut self.cur, next);
        self.prev = Some((old.recv.clone(), old.number, now_ms + PREVIOUS_EPOCH_GRACE_MS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Session, Session) {
        let root = Zeroizing::new([9u8; 32]);
        let a = Established {
            root: root.clone(),
            hash: [1; 32],
            sas: None,
            initiator: true,
        };
        let b = Established {
            root,
            hash: [1; 32],
            sas: None,
            initiator: false,
        };
        (Session::new(&a, 0), Session::new(&b, 0))
    }

    #[test]
    fn messages_flow_both_ways() {
        let mut rng = crate::test_rng::rng();
        let (mut a, mut b) = pair();
        for i in 0..20u8 {
            let f = a.seal(lane::CHAT, &[i; 10]).unwrap();
            assert_eq!(
                b.open(&f, 0, &mut rng).unwrap().messages,
                vec![(lane::CHAT, vec![i; 10])]
            );
            let g = b.seal(lane::FILE, &[i; 3000]).unwrap();
            assert_eq!(a.open(&g, 0, &mut rng).unwrap().messages[0].1.len(), 3000);
        }
        assert_eq!(a.contact_secret(), b.contact_secret());
        assert_eq!(a.signal_keys().topic(5), b.signal_keys().topic(5));
    }

    #[test]
    fn replay_reorder_and_forgery() {
        let mut rng = crate::test_rng::rng();
        let (mut a, mut b) = pair();
        let f0 = a.seal(lane::CHAT, b"0").unwrap();
        let f1 = a.seal(lane::CHAT, b"1").unwrap();
        let f2 = a.seal(lane::CHAT, b"2").unwrap();
        assert_eq!(b.open(&f2, 0, &mut rng).unwrap().messages[0].1, b"2");
        assert_eq!(b.open(&f0, 0, &mut rng).unwrap().messages[0].1, b"0");
        assert_eq!(b.open(&f0, 0, &mut rng).unwrap_err(), Error::Replay);
        let mut forged = f1.clone();
        let last = forged.len() - 1;
        forged[last] ^= 1;
        assert_eq!(b.open(&forged, 0, &mut rng).unwrap_err(), Error::Decrypt);
        // The forgery did not consume the key.
        assert_eq!(b.open(&f1, 0, &mut rng).unwrap().messages[0].1, b"1");
        // Moving a frame to another lane breaks its authentication.
        let f3 = a.seal(lane::CHAT, b"3").unwrap();
        let mut moved = f3.clone();
        moved[1] = lane::FILE;
        assert!(b.open(&moved, 0, &mut rng).is_err());
        // Reflection: a frame never decrypts for its own sender.
        assert!(a.open(&f3, 0, &mut rng).is_err());
    }

    #[test]
    fn too_far_ahead_is_refused() {
        let mut rng = crate::test_rng::rng();
        let (mut a, mut b) = pair();
        let mut last = Vec::new();
        for _ in 0..=MAX_SKIP + 1 {
            last = a.seal(lane::CHAT, b"x").unwrap();
        }
        assert_eq!(b.open(&last, 0, &mut rng).unwrap_err(), Error::TooFarAhead);
    }

    #[test]
    fn rekey_rotates_and_keeps_in_flight_frames() {
        let mut rng = crate::test_rng::rng();
        let (mut a, mut b) = pair();
        assert!(a.poll(1000, &mut rng).is_empty());
        let rekey = a.poll(REKEY_INTERVAL_MS + 1, &mut rng);
        assert_eq!(rekey.len(), 1);
        // A frame A sends in the old epoch while the rekey is in flight.
        let old_a = a.seal(lane::CHAT, b"old from a").unwrap();

        let opened = b.open(&rekey[0].1, REKEY_INTERVAL_MS + 2, &mut rng).unwrap();
        assert!(opened.rekeyed);
        assert_eq!(b.epoch(), 1);
        // B speaks in the new epoch before A has the acknowledgement: A holds the frame.
        let new_b = b.seal(lane::CHAT, b"new from b").unwrap();
        assert!(
            a.open(&new_b, REKEY_INTERVAL_MS + 3, &mut rng)
                .unwrap()
                .messages
                .is_empty()
        );
        let done = a.open(&opened.replies[0].1, REKEY_INTERVAL_MS + 4, &mut rng).unwrap();
        assert!(done.rekeyed);
        assert_eq!(done.messages, vec![(lane::CHAT, b"new from b".to_vec())]);
        assert_eq!(a.epoch(), 1);
        // B still reads A's old-epoch frame during the grace period.
        assert_eq!(
            b.open(&old_a, REKEY_INTERVAL_MS + 5, &mut rng).unwrap().messages[0].1,
            b"old from a"
        );
        let new_a = a.seal(lane::CHAT, b"after").unwrap();
        assert_eq!(
            b.open(&new_a, REKEY_INTERVAL_MS + 6, &mut rng).unwrap().messages[0].1,
            b"after"
        );
        // After the grace period the old epoch is gone.
        let late_old = old_a.clone();
        b.poll(REKEY_INTERVAL_MS + 5 + PREVIOUS_EPOCH_GRACE_MS + 10, &mut rng);
        assert!(b.open(&late_old, 0, &mut rng).is_err());
        // The responder cannot start a rekey itself.
        assert!(b.start_rekey(0, &mut rng).is_err());
    }

    #[test]
    fn rekey_by_message_count() {
        let mut rng = crate::test_rng::rng();
        let (mut a, mut b) = pair();
        for _ in 0..REKEY_MESSAGES {
            let f = a.seal(lane::CHAT, b"m").unwrap();
            b.open(&f, 0, &mut rng).unwrap();
        }
        let frames = a.poll(0, &mut rng);
        assert_eq!(frames.len(), 1);
        let opened = b.open(&frames[0].1, 0, &mut rng).unwrap();
        a.open(&opened.replies[0].1, 0, &mut rng).unwrap();
        assert_eq!((a.epoch(), b.epoch()), (1, 1));
    }

    #[test]
    fn bye_erases_both_sides() {
        let mut rng = crate::test_rng::rng();
        let (mut a, mut b) = pair();
        let bye = a.close().unwrap();
        assert!(a.is_closed());
        assert!(a.seal(lane::CHAT, b"x").is_err());
        assert!(b.open(&bye, 0, &mut rng).unwrap().closed);
        assert!(b.is_closed());
    }

    #[test]
    fn padding_hides_short_lengths() {
        let (mut a, _) = pair();
        let x = a.seal(lane::CHAT, b"ok").unwrap();
        let y = a
            .seal(lane::CHAT, b"a somewhat longer message, but still short")
            .unwrap();
        assert_eq!(x.len(), y.len());
    }
}
