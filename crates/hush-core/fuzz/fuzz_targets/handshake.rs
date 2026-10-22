//! Handshake messages from someone holding the link, or not: errors only, never a panic.
#![no_main]

use std::sync::OnceLock;

use getrandom::SysRng;
use hush_core::handshake::{ContactParams, ContactResponderPending, LinkCreatorPending, LinkJoiner};
use hush_core::identity::Identity;
use hush_core::link::{Link, LinkKeys, Mode};
use libfuzzer_sys::fuzz_target;
use rand_core::UnwrapErr;

fn keys() -> &'static (LinkKeys, Identity) {
    static K: OnceLock<(LinkKeys, Identity)> = OnceLock::new();
    K.get_or_init(|| {
        let link = Link::generate(Mode::Group, 0, 60, false, &mut UnwrapErr(SysRng));
        (link.keys(None).unwrap(), Identity::from_seed([3; 32]))
    })
}

fuzz_target!(|data: &[u8]| {
    let (k, me) = keys();
    let mut rng = UnwrapErr(SysRng);
    let _ = LinkCreatorPending::read_hs1(k, data);
    let (joiner, _) = LinkJoiner::start(k, b"", &mut rng);
    let _ = joiner.read_hs2(data);
    let psk = [5; 32];
    let params = ContactParams { psk: &psk, ids: (me.public().id_hash(), &[9; 32]) };
    let _ = ContactResponderPending::read_c1(&params, me, |_| true, data);
});
