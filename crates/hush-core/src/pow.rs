//! Hashcash stamps on unsolicited contact requests: cheap to check, costly to mass-produce.
//! `SHA-256("hush/v1/pow" || challenge || nonce)` must start with `bits` zero bits.

use crate::kdf::sha256;

/// Default difficulty: about a second in WebAssembly on a phone.
pub const DEFAULT_BITS: u32 = 20;

fn leading_zero_bits(h: &[u8; 32]) -> u32 {
    let mut n = 0;
    for &b in h {
        if b == 0 {
            n += 8;
        } else {
            n += b.leading_zeros();
            break;
        }
    }
    n
}

#[must_use]
pub fn check(challenge: &[u8], nonce: u64, bits: u32) -> bool {
    leading_zero_bits(&sha256(&[b"hush/v1/pow", challenge, &nonce.to_be_bytes()])) >= bits
}

/// Finds a stamp. Expected work: 2^bits hashes.
#[must_use]
pub fn solve(challenge: &[u8], bits: u32) -> u64 {
    (0u64..).find(|&n| check(challenge, n, bits)).expect("a stamp exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_and_check() {
        let n = solve(b"challenge", 12);
        assert!(check(b"challenge", n, 12));
        assert_eq!(
            leading_zero_bits(&[
                0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]),
            11
        );
    }
}
