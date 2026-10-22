//! X-Wing hybrid KEM (X25519 + ML-KEM-768, draft-connolly-cfrg-xwing-kem). The shared secret is
//! safe as long as either component holds.

use x_wing::{
    Ciphertext, DecapsulationKeyRejectNonContrib, Decapsulator, Encapsulate, EncapsulationKey, KeyExport,
    TryDecapsulate,
};
use zeroize::Zeroizing;

use crate::{Error, Result, Rng};

/// Encapsulation (public) key length.
pub const EK_LEN: usize = x_wing::ENCAPSULATION_KEY_SIZE;
/// Ciphertext length.
pub const CT_LEN: usize = x_wing::CIPHERTEXT_SIZE;
/// Decapsulation key (seed) length.
pub const SEED_LEN: usize = x_wing::DECAPSULATION_KEY_SIZE;

/// An X-Wing decapsulation key. Rejects ciphertexts whose X25519 part is non-contributory.
/// Zeroized on drop.
#[derive(Clone)]
pub struct KemSecret {
    dk: DecapsulationKeyRejectNonContrib,
    seed: Zeroizing<[u8; SEED_LEN]>,
}

impl core::fmt::Debug for KemSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("KemSecret(..)")
    }
}

impl KemSecret {
    pub fn generate<R: Rng + ?Sized>(rng: &mut R) -> Self {
        Self::from_seed(crate::random(rng))
    }

    #[must_use]
    pub fn from_seed(seed: [u8; SEED_LEN]) -> Self {
        Self {
            dk: DecapsulationKeyRejectNonContrib::from(seed),
            seed: Zeroizing::new(seed),
        }
    }

    #[must_use]
    pub fn seed(&self) -> &[u8; SEED_LEN] {
        &self.seed
    }

    /// The encapsulation key to send to the peer.
    #[must_use]
    pub fn public(&self) -> Vec<u8> {
        self.dk.encapsulation_key().to_bytes().to_vec()
    }

    /// Recovers the shared secret from a ciphertext.
    ///
    /// # Errors
    /// [`Error::Malformed`] for a wrong length, [`Error::Decrypt`] for a non-contributory X25519 share.
    pub fn decapsulate(&self, ct: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        if ct.len() != CT_LEN {
            return Err(Error::Malformed);
        }
        let ct = Ciphertext::try_from(ct).map_err(|_| Error::Malformed)?;
        let ss = self.dk.try_decapsulate(&ct).map_err(|_| Error::Decrypt)?;
        Ok(Zeroizing::new(ss.into()))
    }
}

/// Encapsulates a fresh shared secret to `ek`: returns `(ciphertext, shared_secret)`.
///
/// # Errors
/// [`Error::Malformed`] when `ek` is not a valid X-Wing encapsulation key.
pub fn encapsulate<R: Rng + ?Sized>(ek: &[u8], rng: &mut R) -> Result<(Vec<u8>, Zeroizing<[u8; 32]>)> {
    if ek.len() != EK_LEN {
        return Err(Error::Malformed);
    }
    let ek = EncapsulationKey::try_from(ek).map_err(|_| Error::Malformed)?;
    let (ct, ss) = ek.encapsulate_with_rng(rng);
    Ok((ct.to_vec(), Zeroizing::new(ss.into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_sizes() {
        let mut rng = crate::test_rng::rng();
        let sk = KemSecret::generate(&mut rng);
        let ek = sk.public();
        assert_eq!(ek.len(), EK_LEN);
        let (ct, ss) = encapsulate(&ek, &mut rng).unwrap();
        assert_eq!(ct.len(), CT_LEN);
        assert_eq!(*sk.decapsulate(&ct).unwrap(), *ss);
        // Same seed, same key.
        assert_eq!(KemSecret::from_seed(*sk.seed()).public(), ek);
    }

    #[test]
    fn wrong_key_gives_different_secret() {
        let mut rng = crate::test_rng::rng();
        let a = KemSecret::generate(&mut rng);
        let b = KemSecret::generate(&mut rng);
        let (ct, ss) = encapsulate(&a.public(), &mut rng).unwrap();
        // ML-KEM's implicit rejection: a wrong key yields an unrelated secret, never an error.
        assert_ne!(*b.decapsulate(&ct).unwrap(), *ss);
        assert_eq!(a.decapsulate(&ct[1..]).unwrap_err(), Error::Malformed);
        assert!(encapsulate(&a.public()[1..], &mut rng).is_err());
    }
}
