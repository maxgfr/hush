//! Sealed signaling envelopes: what Nostr relays and WebTorrent trackers carry. They only ever see
//! a topic and ciphertext; sender, recipient, kind, time and length are all inside.
//!
//! ```text
//! envelope = v(1) || nonce(24) || XChaCha(env_key, aad = "hush/v1/env" || v || topic, padded(inner))
//! inner    = ts_ms(u64) || from(16) || to(16, zero = everyone) || kind(1) || body
//! ```

use std::collections::{BTreeSet, HashSet};

use crate::aead::{xopen, xseal};
use crate::{Error, Result, Rng, VERSION};

/// Envelopes older (or further in the future) than this are dropped.
pub const MAX_AGE_MS: u64 = 120_000;
/// Manual (QR / copy-paste) codes take longer to carry across.
pub const MANUAL_MAX_AGE_MS: u64 = 600_000;

/// A peer's per-session random tag (never an identity).
pub type PeerTag = [u8; 16];
/// Addressed to everyone on the topic.
pub const BROADCAST: PeerTag = [0u8; 16];

/// Envelope kinds.
pub mod kind {
    /// Link handshake, joiner → creator.
    pub const HS1: u8 = 1;
    /// Link handshake, creator → joiner.
    pub const HS2: u8 = 2;
    /// Link handshake, joiner → creator.
    pub const HS3: u8 = 3;
    /// A session frame relayed over signaling (reconnection offers, relay fallback).
    pub const SESSION: u8 = 4;
    /// "Link already taken" notice from a direct link's creator.
    pub const TAKEN: u8 = 5;
    /// Presence beacon on a contact topic.
    pub const HERE: u8 = 6;
    /// Contact handshake messages.
    pub const C1: u8 = 7;
    pub const C2: u8 = 8;
    pub const C3: u8 = 9;
    /// Identifier lookup: request for a bundle, and the answer.
    pub const LOOKUP: u8 = 10;
    pub const BUNDLE: u8 = 11;
    /// A contact request to an identifier.
    pub const REQUEST: u8 = 12;
}

/// The decrypted content of an envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub ts_ms: u64,
    pub from: PeerTag,
    pub to: PeerTag,
    pub kind: u8,
    pub body: Vec<u8>,
}

fn aad(topic: &[u8; 32]) -> Vec<u8> {
    let mut a = b"hush/v1/env".to_vec();
    a.push(VERSION);
    a.extend_from_slice(topic);
    a
}

/// Seals an envelope for a topic.
pub fn seal<R: Rng + ?Sized>(key: &[u8; 32], topic: &[u8; 32], env: &Envelope, rng: &mut R) -> Vec<u8> {
    let mut inner = Vec::with_capacity(41 + env.body.len());
    inner.extend_from_slice(&env.ts_ms.to_be_bytes());
    inner.extend_from_slice(&env.from);
    inner.extend_from_slice(&env.to);
    inner.push(env.kind);
    inner.extend_from_slice(&env.body);
    let mut out = vec![VERSION];
    out.extend_from_slice(&xseal(key, &aad(topic), &crate::pad::pad(&inner), rng));
    out
}

/// Opens an envelope (no freshness or replay checks: see [`ReplayGuard`]).
///
/// # Errors
/// [`Error::Malformed`] or [`Error::Decrypt`] for anything not sealed with this key and topic.
pub fn open(key: &[u8; 32], topic: &[u8; 32], data: &[u8]) -> Result<Envelope> {
    let (&v, rest) = data.split_first().ok_or(Error::Malformed)?;
    if v != VERSION {
        return Err(Error::Malformed);
    }
    let padded = xopen(key, &aad(topic), rest)?;
    let inner = crate::pad::unpad(&padded)?;
    if inner.len() < 41 {
        return Err(Error::Malformed);
    }
    Ok(Envelope {
        ts_ms: u64::from_be_bytes(inner[..8].try_into().map_err(|_| Error::Malformed)?),
        from: inner[8..24].try_into().map_err(|_| Error::Malformed)?,
        to: inner[24..40].try_into().map_err(|_| Error::Malformed)?,
        kind: inner[40],
        body: inner[41..].to_vec(),
    })
}

/// Drops stale and replayed envelopes. Keyed by the envelope's nonce, which is unique per seal.
///
/// A nonce is remembered until its envelope could no longer pass the freshness check, even through
/// the longer window of manual codes, so a replay is caught however many envelopes arrive in
/// between. When that many fresh envelopes are pending, new ones are refused rather than letting a
/// flood push old nonces out.
#[derive(Debug)]
pub struct ReplayGuard {
    seen: HashSet<[u8; 24]>,
    by_expiry: BTreeSet<(u64, [u8; 24])>,
    capacity: usize,
}

impl Default for ReplayGuard {
    fn default() -> Self {
        Self::new(16_384)
    }
}

impl ReplayGuard {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            seen: HashSet::new(),
            by_expiry: BTreeSet::new(),
            capacity,
        }
    }

    /// Accepts an envelope once, if fresh enough.
    ///
    /// # Errors
    /// [`Error::Expired`] when outside the window, [`Error::Replay`] when already seen,
    /// [`Error::TooFarAhead`] when the guard is full of envelopes still fresh.
    pub fn check(&mut self, data: &[u8], env: &Envelope, now_ms: u64, max_age_ms: u64) -> Result<()> {
        if env.ts_ms.saturating_add(max_age_ms) < now_ms || env.ts_ms > now_ms.saturating_add(max_age_ms) {
            return Err(Error::Expired);
        }
        let nonce: [u8; 24] = data
            .get(1..25)
            .ok_or(Error::Malformed)?
            .try_into()
            .map_err(|_| Error::Malformed)?;
        while let Some(&(expiry, old)) = self.by_expiry.first()
            && expiry < now_ms
        {
            self.by_expiry.pop_first();
            self.seen.remove(&old);
        }
        if self.seen.contains(&nonce) {
            return Err(Error::Replay);
        }
        if self.seen.len() >= self.capacity {
            return Err(Error::TooFarAhead);
        }
        let expiry = env.ts_ms.saturating_add(max_age_ms.max(MANUAL_MAX_AGE_MS));
        self.seen.insert(nonce);
        self.by_expiry.insert((expiry, nonce));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_and_guard() {
        let mut rng = crate::test_rng::rng();
        let key = [1u8; 32];
        let topic = [2u8; 32];
        let env = Envelope {
            ts_ms: 1000,
            from: [3; 16],
            to: BROADCAST,
            kind: kind::HS1,
            body: vec![9; 1300],
        };
        let sealed = seal(&key, &topic, &env, &mut rng);
        let opened = open(&key, &topic, &sealed).unwrap();
        assert_eq!(opened, env);
        assert!(open(&key, &[3u8; 32], &sealed).is_err(), "bound to its topic");
        assert!(open(&[0u8; 32], &topic, &sealed).is_err());

        let mut guard = ReplayGuard::new(2);
        guard.check(&sealed, &opened, 1000, MAX_AGE_MS).unwrap();
        assert_eq!(guard.check(&sealed, &opened, 1000, MAX_AGE_MS), Err(Error::Replay));
        // A flood of fresh envelopes cannot push a remembered nonce out.
        let other = seal(&key, &topic, &env, &mut rng);
        guard.check(&other, &opened, 1000, MAX_AGE_MS).unwrap();
        let third = seal(&key, &topic, &env, &mut rng);
        assert_eq!(guard.check(&third, &opened, 1000, MAX_AGE_MS), Err(Error::TooFarAhead));
        assert_eq!(guard.check(&sealed, &opened, 1000, MAX_AGE_MS), Err(Error::Replay));
        // Once they can no longer be fresh, their nonces are forgotten.
        let later = Envelope {
            ts_ms: 1000 + MANUAL_MAX_AGE_MS + 5,
            ..env.clone()
        };
        let fresh = seal(&key, &topic, &later, &mut rng);
        guard.check(&fresh, &later, later.ts_ms, MAX_AGE_MS).unwrap();
        assert_eq!(
            guard.check(&sealed, &opened, 1000 + MAX_AGE_MS + 1, MAX_AGE_MS),
            Err(Error::Expired)
        );

        // Lengths are hidden: a short and a medium body seal to the same size.
        let a = seal(
            &key,
            &topic,
            &Envelope {
                body: vec![0; 10],
                ..env.clone()
            },
            &mut rng,
        );
        let b = seal(
            &key,
            &topic,
            &Envelope {
                body: vec![0; 150],
                ..env
            },
            &mut rng,
        );
        assert_eq!(a.len(), b.len());
    }
}
