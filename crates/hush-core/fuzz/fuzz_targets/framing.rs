//! Stream records and BLE packets, as a nearby device could send them.
#![no_main]

use hush_core::stream::{Fragmenter, Reassembler, StreamDecoder, encode_record};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&cut, rest)) = data.split_first() else { return };
    // Arbitrary bytes, split anywhere.
    let mut dec = StreamDecoder::new();
    for chunk in rest.chunks(usize::from(cut).max(1)) {
        if dec.push(chunk).is_err() {
            break;
        }
    }
    let mut re = Reassembler::new();
    for packet in rest.chunks(usize::from(cut).max(3)) {
        let _ = re.push(packet);
    }
    // Round trip: whatever goes in comes out.
    let mtu = 20 + usize::from(cut);
    let mut frag = Fragmenter::new(mtu).expect("mtu >= 20");
    let mut re = Reassembler::new();
    let mut out = None;
    for p in frag.split(cut, rest) {
        assert!(p.len() <= mtu);
        if let Some(r) = re.push(&p).expect("in order") {
            out = Some(r);
        }
    }
    assert_eq!(out, Some((cut, rest.to_vec())));
    let mut dec = StreamDecoder::new();
    assert_eq!(dec.push(&encode_record(cut, rest)).expect("valid"), vec![(cut, rest.to_vec())]);
});
