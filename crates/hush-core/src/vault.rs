//! Encrypted local storage. Nothing is persisted unless the user turns history or contacts on,
//! and then only behind a passphrase.
//!
//! ```text
//! header = "HUSHVT" || v || m_kib(u32) || t(u32) || p(u32) || salt(16) || XChaCha(KEK, aad = header prefix, DEK)
//! KEK    = Argon2id(NFKC(passphrase), salt, m, t, p)
//! record = XChaCha(DEK_rec, aad = "hush/v1/vault/rec" || kind || 0 || id, plaintext)
//! index  = HMAC(DEK_idx, kind || 0 || id)[..16]    storage keys never reveal contact ids
//! ```

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::aead::{TAG_LEN, XNONCE_LEN, xopen, xseal};
use crate::kdf::{expand32, hmac};
use crate::{Error, Result, Rng, VERSION};

const MAGIC: &[u8; 6] = b"HUSHVT";
const HEADER_PREFIX_LEN: usize = 6 + 1 + 12 + 16;

/// Argon2id costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    /// Passes.
    pub t: u32,
    /// Lanes.
    pub p: u32,
}

impl KdfParams {
    /// Default for the vault and backups: 64 MiB, 3 passes (about a second in WebAssembly).
    pub const DEFAULT: KdfParams = KdfParams {
        m_kib: 64 * 1024,
        t: 3,
        p: 1,
    };

    fn validate(self) -> Result<Self> {
        // Refuse costs a crafted backup could use to exhaust memory or time (a phone is killed
        // well before 1 GiB): at most 4 times the default memory, and 8 passes. Argon2 itself
        // rejects costs below its minimums.
        if self.m_kib > 256 * 1024 || self.t == 0 || self.t > 8 || self.p == 0 || self.p > 8 {
            return Err(Error::InvalidParameter);
        }
        Ok(self)
    }

    pub(crate) fn encode(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.m_kib.to_be_bytes());
        out.extend_from_slice(&self.t.to_be_bytes());
        out.extend_from_slice(&self.p.to_be_bytes());
    }

    pub(crate) fn decode(b: &[u8]) -> Result<Self> {
        let word = |i: usize| -> Result<u32> {
            Ok(u32::from_be_bytes(
                b.get(i..i + 4)
                    .ok_or(Error::Malformed)?
                    .try_into()
                    .map_err(|_| Error::Malformed)?,
            ))
        };
        Self {
            m_kib: word(0)?,
            t: word(4)?,
            p: word(8)?,
        }
        .validate()
    }
}

/// Argon2id(password, salt) → 32 bytes.
///
/// # Errors
/// [`Error::InvalidParameter`] for out-of-range costs or a short salt.
pub fn argon2id(password: &[u8], salt: &[u8], params: KdfParams) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(params.m_kib, params.t, params.p, Some(32)).map_err(|_| Error::InvalidParameter)?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, &mut *out)
        .map_err(|_| Error::InvalidParameter)?;
    Ok(out)
}

/// NFKC normalization of a passphrase, so the same words typed on two keyboards give the same key.
/// Without the `nfkc` feature the host must normalize before calling (the web app does).
#[must_use]
pub fn normalize(passphrase: &str) -> Zeroizing<String> {
    #[cfg(feature = "nfkc")]
    {
        use unicode_normalization::UnicodeNormalization;
        Zeroizing::new(passphrase.nfkc().collect())
    }
    #[cfg(not(feature = "nfkc"))]
    {
        Zeroizing::new(passphrase.to_string())
    }
}

pub(crate) fn passphrase_key(passphrase: &str, salt: &[u8], params: KdfParams) -> Result<Zeroizing<[u8; 32]>> {
    let normalized = normalize(passphrase);
    argon2id(normalized.as_bytes(), salt, params.validate()?)
}

/// An unlocked vault: holds the data-encryption key in memory.
pub struct Vault {
    rec_key: Zeroizing<[u8; 32]>,
    idx_key: Zeroizing<[u8; 32]>,
    dek: Zeroizing<[u8; 32]>,
}

impl core::fmt::Debug for Vault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Vault(..)")
    }
}

fn record_aad(kind: &str, id: &str) -> Vec<u8> {
    let mut aad = b"hush/v1/vault/rec".to_vec();
    aad.extend_from_slice(kind.as_bytes());
    aad.push(0);
    aad.extend_from_slice(id.as_bytes());
    aad
}

impl Vault {
    fn from_dek(dek: Zeroizing<[u8; 32]>) -> Self {
        Self {
            rec_key: Zeroizing::new(expand32(&dek, b"hush/v1/vault/records", &[])),
            idx_key: Zeroizing::new(expand32(&dek, b"hush/v1/vault/index", &[])),
            dek,
        }
    }

    /// Creates a vault with a fresh key; returns it with the header to store.
    ///
    /// # Errors
    /// [`Error::InvalidParameter`] for bad Argon2 costs.
    pub fn create<R: Rng + ?Sized>(passphrase: &str, params: KdfParams, rng: &mut R) -> Result<(Self, Vec<u8>)> {
        let vault = Self::from_dek(Zeroizing::new(crate::random(rng)));
        let header = vault.header(passphrase, params, rng)?;
        Ok((vault, header))
    }

    /// Wraps the key under a (new) passphrase; used on creation and to change the passphrase.
    ///
    /// # Errors
    /// [`Error::InvalidParameter`] for bad Argon2 costs.
    pub fn header<R: Rng + ?Sized>(&self, passphrase: &str, params: KdfParams, rng: &mut R) -> Result<Vec<u8>> {
        let salt: [u8; 16] = crate::random(rng);
        let kek = passphrase_key(passphrase, &salt, params)?;
        let mut header = MAGIC.to_vec();
        header.push(VERSION);
        params.encode(&mut header);
        header.extend_from_slice(&salt);
        let wrapped = xseal(&kek, &header, &*self.dek, rng);
        header.extend_from_slice(&wrapped);
        Ok(header)
    }

    /// Unlocks a vault from its header.
    ///
    /// # Errors
    /// [`Error::Decrypt`] for a wrong passphrase, [`Error::Malformed`] for a corrupt header.
    pub fn unlock(passphrase: &str, header: &[u8]) -> Result<Self> {
        if header.len() != HEADER_PREFIX_LEN + XNONCE_LEN + 32 + TAG_LEN
            || &header[..6] != MAGIC
            || header[6] != VERSION
        {
            return Err(Error::Malformed);
        }
        let params = KdfParams::decode(&header[7..19])?;
        let salt = &header[19..35];
        let kek = passphrase_key(passphrase, salt, params)?;
        let dek = xopen(&kek, &header[..HEADER_PREFIX_LEN], &header[HEADER_PREFIX_LEN..])?;
        let dek: [u8; 32] = dek.as_slice().try_into().map_err(|_| Error::Malformed)?;
        Ok(Self::from_dek(Zeroizing::new(dek)))
    }

    /// Encrypts a record.
    pub fn seal<R: Rng + ?Sized>(&self, kind: &str, id: &str, plaintext: &[u8], rng: &mut R) -> Vec<u8> {
        xseal(&self.rec_key, &record_aad(kind, id), &crate::pad::pad(plaintext), rng)
    }

    /// Decrypts a record; fails if it was moved to another kind or id.
    ///
    /// # Errors
    /// [`Error::Decrypt`] when tampered with or misplaced.
    pub fn open(&self, kind: &str, id: &str, blob: &[u8]) -> Result<Vec<u8>> {
        let padded = xopen(&self.rec_key, &record_aad(kind, id), blob)?;
        Ok(crate::pad::unpad(&padded)?.to_vec())
    }

    /// Opaque storage key for a record (hex, 32 chars).
    #[must_use]
    pub fn index(&self, kind: &str, id: &str) -> String {
        let mac = hmac(&*self.idx_key, &[kind.as_bytes(), &[0], id.as_bytes()]);
        mac[..16].iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const CHEAP: KdfParams = KdfParams { m_kib: 64, t: 1, p: 1 };

    #[test]
    fn create_unlock_and_records() {
        let mut rng = crate::test_rng::rng();
        let (vault, header) = Vault::create("passphrase", CHEAP, &mut rng).unwrap();
        let blob = vault.seal("contact", "abc", b"alice", &mut rng);
        let again = Vault::unlock("passphrase", &header).unwrap();
        assert_eq!(again.open("contact", "abc", &blob).unwrap(), b"alice");
        assert_eq!(again.index("contact", "abc"), vault.index("contact", "abc"));
        assert_ne!(vault.index("contact", "abc"), vault.index("contact", "abd"));
        assert_eq!(again.open("contact", "abd", &blob), Err(Error::Decrypt));
        assert_eq!(Vault::unlock("wrong", &header).unwrap_err(), Error::Decrypt);

        // Changing the passphrase keeps the same records readable.
        let header2 = again.header("new pass", CHEAP, &mut rng).unwrap();
        let third = Vault::unlock("new pass", &header2).unwrap();
        assert_eq!(third.open("contact", "abc", &blob).unwrap(), b"alice");

        let mut bad = header.clone();
        bad[8] ^= 1; // tamper with the costs
        assert!(Vault::unlock("passphrase", &bad).is_err());
    }

    #[test]
    fn costs_a_phone_cannot_pay_are_refused() {
        assert!(KdfParams::DEFAULT.validate().is_ok());
        let big = KdfParams {
            m_kib: 256 * 1024 + 1,
            ..KdfParams::DEFAULT
        };
        assert!(big.validate().is_err());
        let slow = KdfParams {
            t: 9,
            ..KdfParams::DEFAULT
        };
        assert!(slow.validate().is_err());
    }
}
