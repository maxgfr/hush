//! HKDF-SHA-256, HMAC-SHA-256 and SHA-256 helpers. Every label is a full byte string starting
//! with `hush/v1/`, so no two derivations in the protocol can collide.

use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

/// SHA-256 over the concatenation of `parts`.
#[must_use]
pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// HKDF-Extract(salt, ikm).
#[must_use]
pub fn extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    let (prk, _) = Hkdf::<Sha256>::extract(Some(salt), ikm);
    prk.into()
}

/// HKDF-Expand(prk, label || extra..., L) into `out`.
///
/// # Panics
/// Only if `out` is longer than 255 * 32 bytes, which no caller does.
pub fn expand_into(prk: &[u8; 32], label: &[u8], extra: &[&[u8]], out: &mut [u8]) {
    let hk = Hkdf::<Sha256>::from_prk(prk).expect("32-byte PRK");
    let mut info: Vec<&[u8]> = Vec::with_capacity(extra.len() + 1);
    info.push(label);
    info.extend_from_slice(extra);
    hk.expand_multi_info(&info, out)
        .expect("output length within HKDF bounds");
}

/// HKDF-Expand to 32 bytes.
#[must_use]
pub fn expand32(prk: &[u8; 32], label: &[u8], extra: &[&[u8]]) -> [u8; 32] {
    let mut out = [0u8; 32];
    expand_into(prk, label, extra, &mut out);
    out
}

/// HKDF-Expand to 16 bytes.
#[must_use]
pub fn expand16(prk: &[u8; 32], label: &[u8], extra: &[&[u8]]) -> [u8; 16] {
    let mut out = [0u8; 16];
    expand_into(prk, label, extra, &mut out);
    out
}

/// HMAC-SHA-256(key, parts...).
#[must_use]
pub fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

/// Constant-time equality for secrets of equal length.
#[must_use]
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn hkdf_rfc5869_case_1() {
        let ikm = [0x0bu8; 22];
        let salt: Vec<u8> = (0x00..=0x0c).collect();
        let info: Vec<u8> = (0xf0..=0xf9).collect();
        let prk = extract(&salt, &ikm);
        assert_eq!(
            prk.to_vec(),
            hex("077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5")
        );
        let mut okm = [0u8; 42];
        expand_into(&prk, &info, &[], &mut okm);
        assert_eq!(
            okm.to_vec(),
            hex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")
        );
    }

    #[test]
    fn sha256_and_hmac_known_answers() {
        assert_eq!(
            sha256(&[b"ab", b"c"]).to_vec(),
            hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        // RFC 4231 test case 2.
        assert_eq!(
            hmac(b"Jefe", &[b"what do ya want ", b"for nothing?"]).to_vec(),
            hex("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")
        );
    }

    #[test]
    fn ct_eq_works() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    pub(crate) fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
