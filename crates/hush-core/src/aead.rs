//! ChaCha20-Poly1305 (wire, one key per message or explicit counter nonce) and
//! XChaCha20-Poly1305 (storage and envelopes, random 24-byte nonce).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, XChaCha20Poly1305};

use crate::{Error, Result, Rng};

/// Poly1305 tag length.
pub const TAG_LEN: usize = 16;
/// XChaCha20 nonce length.
pub const XNONCE_LEN: usize = 24;

/// The 96-bit nonce for counter `n`: four zero bytes then `n` big-endian.
#[must_use]
pub fn counter_nonce(n: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&n.to_be_bytes());
    nonce
}

/// ChaCha20-Poly1305 encryption.
#[must_use]
pub fn seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], pt: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new_from_slice(key).expect("32-byte key");
    cipher
        .encrypt(&(*nonce).into(), Payload { msg: pt, aad })
        .expect("ChaCha20-Poly1305 encryption of a bounded message")
}

/// ChaCha20-Poly1305 decryption.
///
/// # Errors
/// [`Error::Decrypt`] when the tag does not verify.
pub fn open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new_from_slice(key).expect("32-byte key");
    cipher
        .decrypt(&(*nonce).into(), Payload { msg: ct, aad })
        .map_err(|_| Error::Decrypt)
}

/// XChaCha20-Poly1305 with a fresh random nonce: returns `nonce(24) || ciphertext`.
#[must_use]
pub fn xseal<R: Rng + ?Sized>(key: &[u8; 32], aad: &[u8], pt: &[u8], rng: &mut R) -> Vec<u8> {
    let nonce: [u8; XNONCE_LEN] = crate::random(rng);
    let mut out = nonce.to_vec();
    out.extend_from_slice(&xseal_with_nonce(key, &nonce, aad, pt));
    out
}

/// XChaCha20-Poly1305 with an explicit nonce (only for known-answer tests and callers that
/// manage nonces themselves).
#[must_use]
pub fn xseal_with_nonce(key: &[u8; 32], nonce: &[u8; XNONCE_LEN], aad: &[u8], pt: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).expect("32-byte key");
    cipher
        .encrypt(&(*nonce).into(), Payload { msg: pt, aad })
        .expect("XChaCha20-Poly1305 encryption of a bounded message")
}

/// Opens `nonce(24) || ciphertext` produced by [`xseal`].
///
/// # Errors
/// [`Error::Malformed`] when too short, [`Error::Decrypt`] when the tag does not verify.
pub fn xopen(key: &[u8; 32], aad: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < XNONCE_LEN + TAG_LEN {
        return Err(Error::Malformed);
    }
    let (nonce, ct) = data.split_at(XNONCE_LEN);
    let nonce: [u8; XNONCE_LEN] = nonce.try_into().map_err(|_| Error::Malformed)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key).expect("32-byte key");
    cipher
        .decrypt(&nonce.into(), Payload { msg: ct, aad })
        .map_err(|_| Error::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kdf::tests::hex;

    #[test]
    fn chacha20poly1305_rfc8439() {
        let key: [u8; 32] = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")
            .try_into()
            .unwrap();
        let nonce: [u8; 12] = hex("070000004041424344454647").try_into().unwrap();
        let aad = hex("50515253c0c1c2c3c4c5c6c7");
        let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let ct = seal(&key, &nonce, &aad, pt);
        assert_eq!(&ct[ct.len() - 16..], &hex("1ae10b594f09e26a7e902ecbd0600691")[..]);
        assert_eq!(open(&key, &nonce, &aad, &ct).unwrap(), pt.to_vec());
    }

    #[test]
    fn tamper_is_rejected() {
        let mut rng = crate::test_rng::rng();
        let key = [7u8; 32];
        let mut ct = xseal(&key, b"aad", b"hello", &mut rng);
        assert_eq!(xopen(&key, b"aad", &ct).unwrap(), b"hello");
        assert_eq!(xopen(&key, b"other", &ct), Err(Error::Decrypt));
        let last = ct.len() - 1;
        ct[last] ^= 1;
        assert_eq!(xopen(&key, b"aad", &ct), Err(Error::Decrypt));
        assert_eq!(xopen(&key, b"aad", &ct[..10]), Err(Error::Malformed));
    }
}
