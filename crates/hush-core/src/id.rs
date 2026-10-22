//! Self-certifying identifiers: `hush1` + bech32m(version 0 || SHA-256 of the signing keys).
//!
//! No registration: an identifier is computed locally from the identity's public keys, so whoever
//! presents keys that hash to it is its owner. Forging one needs a SHA-256 second preimage.
//! The bech32m checksum catches typos (any 4 wrong characters are detected).

use bech32::primitives::decode::CheckedHrpstring;
use bech32::{Bech32m, Hrp};

use crate::{Error, Result};

const HRP: &str = "hush";
const ID_VERSION: u8 = 0;

/// Encodes an identifier hash as `hush1…` (64 characters).
#[must_use]
pub fn encode(id_hash: &[u8; 32]) -> String {
    let mut data = Vec::with_capacity(33);
    data.push(ID_VERSION);
    data.extend_from_slice(id_hash);
    bech32::encode::<Bech32m>(Hrp::parse_unchecked(HRP), &data).expect("33 bytes fit in bech32m")
}

/// Parses a `hush1…` identifier (case-insensitive, surrounding whitespace ignored).
///
/// # Errors
/// [`Error::Malformed`] for a wrong prefix, checksum, version or length.
pub fn decode(s: &str) -> Result<[u8; 32]> {
    let s = s.trim();
    let checked = CheckedHrpstring::new::<Bech32m>(s).map_err(|_| Error::Malformed)?;
    if checked.hrp().as_str().to_ascii_lowercase() != HRP {
        return Err(Error::Malformed);
    }
    let data: Vec<u8> = checked.byte_iter().collect();
    match data.split_first() {
        Some((&ID_VERSION, rest)) => rest.try_into().map_err(|_| Error::Malformed),
        _ => Err(Error::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let h = [0xabu8; 32];
        let id = encode(&h);
        assert!(id.starts_with("hush1"));
        assert_eq!(id.len(), 64);
        assert_eq!(decode(&id).unwrap(), h);
        assert_eq!(decode(&format!("  {}  ", id.to_uppercase())).unwrap(), h);
    }

    #[test]
    fn typos_are_caught() {
        let id = encode(&[7u8; 32]);
        let mut chars: Vec<char> = id.chars().collect();
        chars[20] = if chars[20] == 'q' { 'p' } else { 'q' };
        let typo: String = chars.into_iter().collect();
        assert!(decode(&typo).is_err());
        assert!(decode(&id.replacen("hush", "husk", 1)).is_err());
        assert!(decode("hush1").is_err());
    }
}
