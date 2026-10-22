//! Sans-IO protocol core of hush.
//!
//! Everything cryptographic lives here, shared by the web app (through `hush-wasm`), the CLI and
//! later the mobile apps (through UniFFI). The core never touches the network, the disk or the
//! clock: hosts pass bytes in, get bytes out, and hand it `now_ms` and a CSPRNG.
//!
//! One cipher suite, no negotiation:
//! - key exchange: X-Wing (X25519 + ML-KEM-768),
//! - signatures: Ed25519 AND ML-DSA-65 (both must verify),
//! - AEAD: ChaCha20-Poly1305 on the wire, XChaCha20-Poly1305 at rest,
//! - KDF: HKDF-SHA-256 / HMAC-SHA-256, Argon2id for passphrases.
//!
//! See `PROTOCOL.md` at the repository root for the byte-level specification.
#![forbid(unsafe_code)]

pub mod aead;
pub mod b64;
pub mod backup;
pub mod bundle;
pub mod contact;
pub mod envelope;
pub mod error;
pub mod handshake;
pub mod id;
pub mod identity;
pub mod kdf;
pub mod kem;
pub mod link;
pub mod lookup;
pub mod noise;
pub mod pad;
pub mod pow;
pub mod sas;
pub mod session;
pub mod vault;

pub use error::{Error, Result};

/// The protocol version carried by every versioned structure.
pub const VERSION: u8 = 1;

/// A CSPRNG as the core expects it (rand_core 0.10, infallible).
pub trait Rng: rand_core::CryptoRng {}
impl<R: rand_core::CryptoRng + ?Sized> Rng for R {}

/// Fills a fresh array with random bytes.
pub(crate) fn random<const N: usize, R: Rng + ?Sized>(rng: &mut R) -> [u8; N] {
    let mut out = [0u8; N];
    rng.fill_bytes(&mut out);
    out
}

/// Days since the Unix epoch (UTC), the unit of every rotating rendezvous topic.
#[must_use]
pub fn utc_day(now_ms: u64) -> u32 {
    u32::try_from(now_ms / 86_400_000).unwrap_or(u32::MAX)
}

#[cfg(test)]
pub(crate) mod test_rng {
    use rand_core::UnwrapErr;

    pub fn rng() -> UnwrapErr<getrandom::SysRng> {
        UnwrapErr(getrandom::SysRng)
    }
}
