//! Small helpers shared by the CLI modules.

use getrandom::SysRng;
use rand_core::UnwrapErr;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The CSPRNG handed to the core.
pub fn rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand_core::Rng::fill_bytes(&mut rng(), &mut out);
    out
}

pub fn b64u(data: &[u8]) -> String {
    hush_core::b64::encode(data)
}

pub fn from_b64u(s: &str) -> Option<Vec<u8>> {
    hush_core::b64::decode(s).ok()
}

/// A random message id: 12 bytes, 16 base64url characters (also the width of file chunk ids).
pub fn new_id() -> String {
    b64u(&random::<12>())
}
