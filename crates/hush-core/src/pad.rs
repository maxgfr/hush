//! Length hiding. Plaintexts are framed as `len(u32 BE) || body || zeros` and padded to the
//! Padmé length (at most ~12% overhead, leaks O(log log L) bits), never below 256 bytes.

use crate::{Error, Result};

/// Smallest padded size: every short message looks the same.
pub const MIN_PADDED: usize = 256;

/// Padmé (Nikitin et al., PETS 2019).
#[must_use]
pub fn padme(len: usize) -> usize {
    if len < 2 {
        return len;
    }
    let e = usize::BITS - 1 - len.leading_zeros(); // floor(log2 len)
    let s = u32::BITS - e.leading_zeros(); // floor(log2 e) + 1
    let last_bits = e - s;
    let mask = (1usize << last_bits) - 1;
    (len + mask) & !mask
}

/// Frames and pads `body`.
#[must_use]
pub fn pad(body: &[u8]) -> Vec<u8> {
    let framed = 4 + body.len();
    let target = padme(framed.max(MIN_PADDED));
    let mut out = Vec::with_capacity(target);
    out.extend_from_slice(&u32::try_from(body.len()).expect("body under 4 GiB").to_be_bytes());
    out.extend_from_slice(body);
    out.resize(target, 0);
    out
}

/// Removes framing and padding, checking the padding is all zeros.
///
/// # Errors
/// [`Error::Malformed`] when the length prefix is inconsistent or padding is not zero.
pub fn unpad(data: &[u8]) -> Result<&[u8]> {
    let len_bytes: [u8; 4] = data
        .get(..4)
        .ok_or(Error::Malformed)?
        .try_into()
        .map_err(|_| Error::Malformed)?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let body = data.get(4..4 + len).ok_or(Error::Malformed)?;
    if data[4 + len..].iter().any(|&b| b != 0) {
        return Err(Error::Malformed);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padme_values() {
        assert_eq!(padme(256), 256);
        assert_eq!(padme(257), 272);
        assert_eq!(padme(1000), 1024);
        assert_eq!(padme(64_520), 65_536);
        for len in 2..100_000 {
            let p = padme(len);
            assert!(p >= len);
            assert!((p - len) as f64 <= len as f64 * 0.12 + 1.0, "len {len} -> {p}");
        }
    }

    #[test]
    fn round_trip() {
        for len in [0usize, 1, 5, 251, 252, 253, 1000, 70_000] {
            let body: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let padded = pad(&body);
            assert!(padded.len() >= MIN_PADDED);
            assert_eq!(unpad(&padded).unwrap(), &body[..]);
        }
        assert_eq!(pad(b"a").len(), pad(b"a much longer message, still short").len());
        let mut bad = pad(b"x");
        bad[100] = 1;
        assert!(unpad(&bad).is_err());
        assert!(unpad(&[0, 0, 1, 0]).is_err());
    }
}
