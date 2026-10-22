//! Reaching someone from their identifier alone, with no directory.
//!
//! Everything derives from the identifier hash, so only people who know the identifier can find
//! the topic or read what is posted there:
//! ```text
//! lroot   = Extract("hush/v1/lookup", id_hash)
//! topic_d = Expand(lroot, "hush/v1/lookup/topic" || day)
//! env_key = Expand(lroot, "hush/v1/lookup/env")
//! psk     = Expand(lroot, "hush/v1/lookup/psk")      contact handshake key for a first request
//! ```
//! The owner, when "reachable" is on, answers `LOOKUP` with its bundle. A request is then a
//! contact handshake `C1` (carrying the requester's bundle) plus a proof of work over
//! `id_hash || day || SHA-256(C1)`. Nothing reaches the network from the owner until they accept.

use crate::kdf::{expand32, extract, sha256};

#[derive(Clone)]
pub struct LookupKeys {
    root: zeroize::Zeroizing<[u8; 32]>,
    id_hash: [u8; 32],
}

impl LookupKeys {
    #[must_use]
    pub fn new(id_hash: &[u8; 32]) -> Self {
        Self {
            root: zeroize::Zeroizing::new(extract(b"hush/v1/lookup", id_hash)),
            id_hash: *id_hash,
        }
    }

    #[must_use]
    pub fn topic(&self, day: u32) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/lookup/topic", &[&day.to_be_bytes()])
    }

    #[must_use]
    pub fn env_key(&self) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/lookup/env", &[])
    }

    #[must_use]
    pub fn psk(&self) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/lookup/psk", &[])
    }

    /// The proof-of-work challenge for a request carrying `c1` on `day`.
    #[must_use]
    pub fn challenge(&self, day: u32, c1: &[u8]) -> Vec<u8> {
        let mut c = self.id_hash.to_vec();
        c.extend_from_slice(&day.to_be_bytes());
        c.extend_from_slice(&sha256(&[c1]));
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_identifier() {
        let a = LookupKeys::new(&[1; 32]);
        let b = LookupKeys::new(&[2; 32]);
        assert_ne!(a.topic(1), b.topic(1));
        assert_ne!(a.challenge(1, b"x"), a.challenge(2, b"x"));
    }
}
