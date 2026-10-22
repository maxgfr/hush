//! Encrypted backup files, readable by the web app and the CLI alike.
//!
//! ```text
//! file = "HUSHBK" || v || m_kib || t || p || salt(16) || nonce(24) || XChaCha(Argon2id(pass), aad = header, padded snapshot)
//! ```
//! The costs travel in the header so a stronger device can choose stronger ones. The snapshot is
//! opaque to the core (the apps use JSON, see `PROTOCOL.md`).

use crate::aead::{xopen, xseal};
use crate::vault::{KdfParams, passphrase_key};
use crate::{Error, Result, Rng, VERSION};

const MAGIC: &[u8; 6] = b"HUSHBK";
const HEADER_LEN: usize = 6 + 1 + 12 + 16;

/// Encrypts a snapshot under a passphrase.
///
/// # Errors
/// [`Error::InvalidParameter`] for bad Argon2 costs.
pub fn export<R: Rng + ?Sized>(passphrase: &str, params: KdfParams, snapshot: &[u8], rng: &mut R) -> Result<Vec<u8>> {
    let salt: [u8; 16] = crate::random(rng);
    let key = passphrase_key(passphrase, &salt, params)?;
    let mut out = MAGIC.to_vec();
    out.push(VERSION);
    params.encode(&mut out);
    out.extend_from_slice(&salt);
    let sealed = xseal(&key, &out, &crate::pad::pad(snapshot), rng);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Decrypts a backup file.
///
/// # Errors
/// [`Error::Malformed`] if it is not a hush backup, [`Error::Decrypt`] for a wrong passphrase or
/// a modified file.
pub fn import(passphrase: &str, file: &[u8]) -> Result<Vec<u8>> {
    if file.len() < HEADER_LEN || &file[..6] != MAGIC || file[6] != VERSION {
        return Err(Error::Malformed);
    }
    let params = KdfParams::decode(&file[7..19])?;
    let key = passphrase_key(passphrase, &file[19..35], params)?;
    let padded = xopen(&key, &file[..HEADER_LEN], &file[HEADER_LEN..])?;
    Ok(crate::pad::unpad(&padded)?.to_vec())
}

/// Whether bytes look like a hush backup (to route a dropped file).
#[must_use]
pub fn is_backup(file: &[u8]) -> bool {
    file.len() > HEADER_LEN && &file[..6] == MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::CHEAP;

    #[test]
    fn round_trip_and_tamper() {
        let mut rng = crate::test_rng::rng();
        let file = export("pass phrase", CHEAP, b"{\"v\":1}", &mut rng).unwrap();
        assert!(is_backup(&file));
        assert_eq!(import("pass phrase", &file).unwrap(), b"{\"v\":1}");
        assert_eq!(import("wrong", &file), Err(Error::Decrypt));
        let mut t = file.clone();
        let n = t.len() - 3;
        t[n] ^= 0x40;
        assert_eq!(import("pass phrase", &t), Err(Error::Decrypt));
        let mut h = file;
        h[20] ^= 1; // salt
        assert_eq!(import("pass phrase", &h), Err(Error::Decrypt));
        assert_eq!(import("x", b"not a backup at all, really not"), Err(Error::Malformed));
    }
}
