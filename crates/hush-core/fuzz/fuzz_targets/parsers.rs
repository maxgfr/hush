//! Every parser of untrusted bytes or text: none may panic, whatever the input.
#![no_main]

use hush_core::{b64, backup, bundle, envelope, id, link, pad};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pad::unpad(data);
    let _ = bundle::Bundle::parse_stored(data);
    let _ = bundle::Bundle::parse(data, 1_750_000_000_000);
    let _ = bundle::verify_card(data, &[7; 32], 1_750_000_000_000);
    let _ = envelope::open(&[1; 32], &[2; 32], data);
    // Backups carry their own Argon2 costs: only cheap ones here, or each input takes seconds.
    if data.len() >= 19 && data[7..11] == [0, 0, 0, 8] && data[11..15] == [0, 0, 0, 1] {
        let _ = backup::import("passphrase", data);
    }
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = link::Link::parse(s);
        let _ = id::decode(s);
        let _ = b64::decode(s);
        let _ = bundle::display_name(s);
    }
});
