//! Long-term identity: one 32-byte seed from which every key is derived.
//!
//! Signatures are hybrid and conjunctive: `Ed25519(m) || ML-DSA-65(m)`, and both must verify.
//! The signed message starts with the signer's identifier hash, so a signature cannot be lifted
//! from one identity to another or stripped down to a single algorithm.

use ed25519_dalek::{Signature as EdSignature, SigningKey as EdSigningKey, VerifyingKey as EdVerifyingKey};
use ml_dsa::{
    EncodedVerifyingKey, MlDsa65, Signature as MlSignature, SigningKey as MlSigningKey, VerifyingKey as MlVerifyingKey,
};
use zeroize::Zeroizing;

use crate::kdf::{expand32, sha256};
use crate::kem::KemSecret;
use crate::{Error, Result, Rng};

pub const ED_PK_LEN: usize = 32;
pub const MLDSA_PK_LEN: usize = 1952;
pub const ED_SIG_LEN: usize = 64;
pub const MLDSA_SIG_LEN: usize = 3309;
/// Length of a hybrid signature.
pub const SIG_LEN: usize = ED_SIG_LEN + MLDSA_SIG_LEN;
/// Signed prekeys rotate every week.
pub const SPK_PERIOD_DAYS: u32 = 7;

const MLDSA_CONTEXT: &[u8] = b"hush/v1";

/// The public half of an identity.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicIdentity {
    ed_pk: [u8; ED_PK_LEN],
    mldsa_pk: Vec<u8>,
    id_hash: [u8; 32],
}

impl core::fmt::Debug for PublicIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PublicIdentity({})", crate::id::encode(&self.id_hash))
    }
}

/// `SHA-256("hush/v1/id" || ed_pk || mldsa_pk)`; fixed-length inputs make it unambiguous.
#[must_use]
pub fn id_hash(ed_pk: &[u8; ED_PK_LEN], mldsa_pk: &[u8]) -> [u8; 32] {
    sha256(&[b"hush/v1/id", ed_pk, mldsa_pk])
}

fn signed_message(id_hash: &[u8; 32], purpose: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(16 + 32 + 1 + purpose.len() + msg.len());
    m.extend_from_slice(b"hush/v1/sig");
    m.extend_from_slice(id_hash);
    m.push(u8::try_from(purpose.len()).expect("short purpose label"));
    m.extend_from_slice(purpose);
    m.extend_from_slice(msg);
    m
}

impl PublicIdentity {
    /// Builds a public identity from raw keys, checking they parse.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a key has the wrong length or encoding.
    pub fn from_keys(ed_pk: &[u8], mldsa_pk: &[u8]) -> Result<Self> {
        let ed_pk: [u8; ED_PK_LEN] = ed_pk.try_into().map_err(|_| Error::Malformed)?;
        EdVerifyingKey::from_bytes(&ed_pk).map_err(|_| Error::Malformed)?;
        if mldsa_pk.len() != MLDSA_PK_LEN {
            return Err(Error::Malformed);
        }
        let id_hash = id_hash(&ed_pk, mldsa_pk);
        Ok(Self {
            ed_pk,
            mldsa_pk: mldsa_pk.to_vec(),
            id_hash,
        })
    }

    #[must_use]
    pub fn ed_pk(&self) -> &[u8; ED_PK_LEN] {
        &self.ed_pk
    }

    #[must_use]
    pub fn mldsa_pk(&self) -> &[u8] {
        &self.mldsa_pk
    }

    #[must_use]
    pub fn id_hash(&self) -> &[u8; 32] {
        &self.id_hash
    }

    /// The human-readable `hush1…` identifier.
    #[must_use]
    pub fn id(&self) -> String {
        crate::id::encode(&self.id_hash)
    }

    /// Verifies a hybrid signature made by [`Identity::sign`] with the same `purpose`.
    ///
    /// # Errors
    /// [`Error::Malformed`] for a wrong length, [`Error::BadSignature`] if either half fails.
    pub fn verify(&self, purpose: &[u8], msg: &[u8], sig: &[u8]) -> Result<()> {
        if sig.len() != SIG_LEN {
            return Err(Error::Malformed);
        }
        let m = signed_message(&self.id_hash, purpose, msg);
        let ed = EdVerifyingKey::from_bytes(&self.ed_pk).map_err(|_| Error::Malformed)?;
        let ed_sig = EdSignature::from_slice(&sig[..ED_SIG_LEN]).map_err(|_| Error::Malformed)?;
        let ed_ok = ed.verify_strict(&m, &ed_sig).is_ok();

        let enc = EncodedVerifyingKey::<MlDsa65>::try_from(self.mldsa_pk.as_slice()).map_err(|_| Error::Malformed)?;
        let ml = MlVerifyingKey::<MlDsa65>::decode(&enc);
        let ml_ok = match MlSignature::<MlDsa65>::try_from(&sig[ED_SIG_LEN..]) {
            Ok(ml_sig) => ml.verify_with_context(&m, MLDSA_CONTEXT, &ml_sig),
            Err(_) => false,
        };
        if ed_ok && ml_ok {
            Ok(())
        } else {
            Err(Error::BadSignature)
        }
    }
}

/// A full identity with its private keys. Zeroized on drop.
pub struct Identity {
    seed: Zeroizing<[u8; 32]>,
    ed: EdSigningKey,
    mldsa: Box<MlSigningKey<MlDsa65>>,
    public: PublicIdentity,
}

impl core::fmt::Debug for Identity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Identity({})", self.public.id())
    }
}

impl Identity {
    pub fn generate<R: Rng + ?Sized>(rng: &mut R) -> Self {
        Self::from_seed(crate::random(rng))
    }

    /// Re-creates an identity from its seed (as stored in the vault or a backup).
    #[must_use]
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let ed_seed = Zeroizing::new(expand32(&seed, b"hush/v1/identity/ed25519", &[]));
        let ml_seed = Zeroizing::new(expand32(&seed, b"hush/v1/identity/mldsa65", &[]));
        let ed = EdSigningKey::from_bytes(&ed_seed);
        let mldsa = Box::new(MlSigningKey::<MlDsa65>::from_seed(&(*ml_seed).into()));
        let ed_pk = ed.verifying_key().to_bytes();
        let mldsa_pk = mldsa.expanded_key().verifying_key().encode().to_vec();
        let public = PublicIdentity {
            id_hash: id_hash(&ed_pk, &mldsa_pk),
            ed_pk,
            mldsa_pk,
        };
        Self {
            seed: Zeroizing::new(seed),
            ed,
            mldsa,
            public,
        }
    }

    #[must_use]
    pub fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    #[must_use]
    pub fn public(&self) -> &PublicIdentity {
        &self.public
    }

    /// Hybrid signature over `msg` for `purpose`. ML-DSA uses hedged (randomized) signing.
    ///
    /// # Panics
    /// Never in practice: ML-DSA only fails for contexts over 255 bytes.
    pub fn sign<R: Rng + ?Sized>(&self, purpose: &[u8], msg: &[u8], rng: &mut R) -> Vec<u8> {
        let m = signed_message(&self.public.id_hash, purpose, msg);
        let mut sig = Vec::with_capacity(SIG_LEN);
        sig.extend_from_slice(&ed25519_dalek::Signer::sign(&self.ed, &m).to_bytes());
        let ml = self
            .mldsa
            .expanded_key()
            .sign_randomized(&m, MLDSA_CONTEXT, rng)
            .expect("short context, infallible RNG");
        sig.extend_from_slice(&ml.encode());
        sig
    }

    /// The signed prekey for a given week (`utc_day / 7`). Derived from the seed, so any epoch can
    /// be recomputed when a contact uses a slightly stale bundle.
    #[must_use]
    pub fn spk(&self, epoch: u32) -> KemSecret {
        KemSecret::from_seed(expand32(
            &self.seed,
            b"hush/v1/identity/xwing-spk",
            &[&epoch.to_be_bytes()],
        ))
    }

    /// The prekey epoch for a given day.
    #[must_use]
    pub fn spk_epoch(utc_day: u32) -> u32 {
        utc_day / SPK_PERIOD_DAYS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_binding() {
        let mut rng = crate::test_rng::rng();
        let alice = Identity::generate(&mut rng);
        let bob = Identity::generate(&mut rng);
        let sig = alice.sign(b"card", b"hello", &mut rng);
        assert_eq!(sig.len(), SIG_LEN);
        alice.public().verify(b"card", b"hello", &sig).unwrap();
        assert_eq!(alice.public().verify(b"card", b"hellO", &sig), Err(Error::BadSignature));
        assert_eq!(
            alice.public().verify(b"bundle", b"hello", &sig),
            Err(Error::BadSignature)
        );
        assert_eq!(bob.public().verify(b"card", b"hello", &sig), Err(Error::BadSignature));

        // Each half alone is not enough.
        let mut only_ed = sig.clone();
        only_ed[ED_SIG_LEN + 10] ^= 1;
        assert_eq!(
            alice.public().verify(b"card", b"hello", &only_ed),
            Err(Error::BadSignature)
        );
        let mut only_ml = sig;
        only_ml[3] ^= 1;
        assert_eq!(
            alice.public().verify(b"card", b"hello", &only_ml),
            Err(Error::BadSignature)
        );
    }

    #[test]
    fn deterministic_from_seed() {
        let a = Identity::from_seed([42u8; 32]);
        let b = Identity::from_seed([42u8; 32]);
        assert_eq!(a.public(), b.public());
        assert_eq!(a.public().mldsa_pk().len(), MLDSA_PK_LEN);
        assert_eq!(a.spk(3).public(), b.spk(3).public());
        assert_ne!(a.spk(3).public(), a.spk(4).public());
        let p = PublicIdentity::from_keys(a.public().ed_pk(), a.public().mldsa_pk()).unwrap();
        assert_eq!(&p, a.public());
    }
}
