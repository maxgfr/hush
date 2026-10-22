//! Unpadded base64url (RFC 4648 §5), the encoding of every secret that travels in a link.

use crate::{Error, Result};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

#[must_use]
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// Decodes unpadded base64url, rejecting padding, other alphabets and non-canonical trailing bits.
///
/// # Errors
/// [`Error::Malformed`] on any deviation.
pub fn decode(s: &str) -> Result<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.len() % 4 == 1 {
        return Err(Error::Malformed);
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'-' => 62,
                b'_' => 63,
                _ => return Err(Error::Malformed),
            };
            n |= u32::from(v) << (18 - 6 * i);
        }
        let produced = chunk.len() - 1;
        let full = n.to_be_bytes();
        out.extend_from_slice(&full[1..=produced]);
        // Leftover bits must be zero so every byte string has exactly one encoding.
        let used_bits = produced * 8;
        let total_bits = chunk.len() * 6;
        if total_bits > used_bits {
            let mask = (1u32 << (24 - used_bits)) - 1;
            if n & mask != 0 {
                return Err(Error::Malformed);
            }
        }
    }
    Ok(out)
}

/// Decodes exactly `N` bytes.
///
/// # Errors
/// [`Error::Malformed`] when the encoding or the length is wrong.
pub fn decode_array<const N: usize>(s: &str) -> Result<[u8; N]> {
    decode(s)?.try_into().map_err(|_| Error::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_length() {
        for len in 0..70 {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(decode(&encode(&data)).unwrap(), data);
        }
        assert_eq!(encode(&[0u8; 32]).len(), 43);
    }

    #[test]
    fn known_values_and_strictness() {
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode(b"fo"), "Zm8");
        assert_eq!(encode(&[0xfb, 0xff]), "-_8");
        assert_eq!(decode("Zm8").unwrap(), b"fo");
        assert!(decode("Zm9").is_err(), "non-zero trailing bits");
        assert!(decode("Zm8=").is_err(), "padding");
        assert!(decode("Zm+8").is_err(), "standard alphabet");
        assert!(decode("Z").is_err());
    }
}
