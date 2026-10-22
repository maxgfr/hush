//! Reconnecting to a saved contact.
//!
//! Both sides keep the contact secret from the session where they exchanged cards. From it:
//! ```text
//! croot   = Extract("hush/v1/contact", secret)
//! topic_d = Expand(croot, "hush/v1/contact/topic" || day)
//! env_key = Expand(croot, "hush/v1/contact/env")
//! ```
//! Whoever comes online posts a `HERE` beacon; the side with the smaller identifier hash then
//! starts the contact handshake, so the two never race.

use crate::kdf::{expand32, extract};

/// Rendezvous keys shared with one contact.
#[derive(Clone)]
pub struct ContactKeys {
    root: zeroize::Zeroizing<[u8; 32]>,
}

impl ContactKeys {
    #[must_use]
    pub fn new(secret: &[u8; 32]) -> Self {
        Self {
            root: zeroize::Zeroizing::new(extract(b"hush/v1/contact", secret)),
        }
    }

    #[must_use]
    pub fn topic(&self, day: u32) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/contact/topic", &[&day.to_be_bytes()])
    }

    #[must_use]
    pub fn env_key(&self) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/contact/env", &[])
    }

    /// The handshake pre-shared key.
    #[must_use]
    pub fn psk(&self) -> [u8; 32] {
        expand32(&self.root, b"hush/v1/contact/psk", &[])
    }
}

/// Whether `me` starts the handshake with `peer`.
#[must_use]
pub fn i_initiate(me: &[u8; 32], peer: &[u8; 32]) -> bool {
    me < peer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_distinct() {
        let k = ContactKeys::new(&[1; 32]);
        assert_ne!(k.topic(1), k.topic(2));
        assert_ne!(k.env_key(), k.psk());
        assert!(i_initiate(&[0; 32], &[1; 32]));
        assert!(!i_initiate(&[1; 32], &[0; 32]));
    }
}
