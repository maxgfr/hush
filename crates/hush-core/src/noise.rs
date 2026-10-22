//! The Noise symmetric state (Noise spec §5.2), with KEM shared secrets mixed in like DH results
//! (as in PQNoise). SHA-256, HKDF and ChaCha20-Poly1305 throughout.

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::aead::{counter_nonce, open, seal};
use crate::kdf::{expand_into, expand32, sha256};
use crate::{Error, Result};

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SymmetricState {
    ck: [u8; 32],
    h: [u8; 32],
    k: Option<[u8; 32]>,
    n: u64,
}

impl SymmetricState {
    #[must_use]
    pub fn new(protocol_name: &[u8]) -> Self {
        let h = sha256(&[protocol_name]);
        Self {
            ck: h,
            h,
            k: None,
            n: 0,
        }
    }

    /// A copy of the state, for handshakes that branch (a group joiner answered by many members).
    #[must_use]
    pub(crate) fn fork(&self) -> Self {
        Self {
            ck: self.ck,
            h: self.h,
            k: self.k,
            n: self.n,
        }
    }

    pub fn mix_hash(&mut self, data: &[u8]) {
        self.h = sha256(&[&self.h, data]);
    }

    /// `ck, k = HKDF(ck, ikm)`; resets the nonce.
    pub fn mix_key(&mut self, ikm: &[u8]) {
        let prk = crate::kdf::extract(&self.ck, ikm);
        let mut out = [0u8; 64];
        expand_into(&prk, b"hush/v1/noise/mix", &[], &mut out);
        self.ck.copy_from_slice(&out[..32]);
        let mut k = [0u8; 32];
        k.copy_from_slice(&out[32..]);
        self.k = Some(k);
        self.n = 0;
        out.zeroize();
    }

    /// Encrypts with the handshake hash as associated data, then hashes the ciphertext.
    ///
    /// # Errors
    /// [`Error::BadState`] before any key has been mixed in.
    pub fn encrypt_and_hash(&mut self, pt: &[u8]) -> Result<Vec<u8>> {
        let k = self.k.ok_or(Error::BadState)?;
        let ct = seal(&k, &counter_nonce(self.n), &self.h, pt);
        self.n += 1;
        self.mix_hash(&ct);
        Ok(ct)
    }

    /// # Errors
    /// [`Error::Decrypt`] when the ciphertext does not authenticate under the transcript so far.
    pub fn decrypt_and_hash(&mut self, ct: &[u8]) -> Result<Vec<u8>> {
        let k = self.k.ok_or(Error::BadState)?;
        let pt = open(&k, &counter_nonce(self.n), &self.h, ct)?;
        self.n += 1;
        self.mix_hash(ct);
        Ok(pt)
    }

    #[must_use]
    pub fn handshake_hash(&self) -> [u8; 32] {
        self.h
    }

    /// The session root once the handshake is over.
    #[must_use]
    pub fn split(&self) -> [u8; 32] {
        expand32(&self.ck, b"hush/v1/noise/split", &[&self.h])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_must_match() {
        let mut a = SymmetricState::new(b"test");
        let mut b = SymmetricState::new(b"test");
        assert!(a.encrypt_and_hash(b"x").is_err());
        a.mix_hash(b"prologue");
        b.mix_hash(b"prologue");
        a.mix_key(b"secret");
        b.mix_key(b"secret");
        let ct = a.encrypt_and_hash(b"hello").unwrap();
        assert_eq!(b.decrypt_and_hash(&ct).unwrap(), b"hello");
        assert_eq!(a.split(), b.split());

        let mut c = SymmetricState::new(b"test");
        c.mix_hash(b"other prologue");
        c.mix_key(b"secret");
        assert!(c.decrypt_and_hash(&ct).is_err());
    }
}
