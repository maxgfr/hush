//! A session driven by arbitrary operations: sends both ways, losses, reordering, replays,
//! corruption, rekeys and time jumps. Everything delivered intact must come out as sent; nothing
//! else may come out, and nothing may panic.
#![no_main]

use getrandom::SysRng;
use hush_core::handshake::Established;
use hush_core::session::{Session, lane};
use libfuzzer_sys::fuzz_target;
use rand_core::UnwrapErr;
use zeroize::Zeroizing;

fuzz_target!(|ops: &[u8]| {
    let mut rng = UnwrapErr(SysRng);
    let est = |initiator| Established { root: Zeroizing::new([9; 32]), hash: [1; 32], sas: None, initiator };
    let mut sides = [Session::new(&est(true), 0), Session::new(&est(false), 0)];
    // Frames in flight towards side 0 and side 1, with the body each must decrypt to.
    let mut wire: [Vec<(Vec<u8>, Option<(u8, Vec<u8>)>)>; 2] = [Vec::new(), Vec::new()];
    let mut now = 0u64;
    for chunk in ops.chunks(3) {
        let [op, a, b] = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let from = usize::from(op & 1);
        let to = 1 - from;
        match op >> 1 & 7 {
            0 | 1 => {
                let l = a % 3;
                let body = vec![b; usize::from(b)];
                if let Ok(f) = sides[from].seal(l, &body) {
                    wire[to].push((f, Some((l, body))));
                }
            }
            2 | 3 => {
                // Deliver one frame (any order); a corrupted copy must be refused.
                let q = &mut wire[from];
                if q.is_empty() {
                    continue;
                }
                let (mut f, expect) = q.remove(usize::from(a) % q.len());
                let corrupt = op & 0x80 != 0 && !f.is_empty();
                if corrupt {
                    let i = usize::from(b) % f.len();
                    f[i] ^= 1 << (a & 7);
                }
                match sides[from].open(&f, now, &mut rng) {
                    // Ok and empty: a frame of the next epoch, held until the rekey completes
                    // (it is authenticated then).
                    Ok(o) => {
                        assert!(!(corrupt && expect.is_some() && !o.messages.is_empty()), "a corrupted frame opened");
                        for r in o.replies {
                            wire[to].push((r.1, None));
                        }
                        if let Some(e) = expect
                            && !o.messages.is_empty()
                        {
                            assert_eq!(o.messages[0], e);
                        }
                    }
                    Err(_) => {}
                }
            }
            4 => {
                // Replay the last frame delivered or not.
                if let Some((f, _)) = wire[from].last().cloned() {
                    let _ = sides[from].open(&f, now, &mut rng);
                    let _ = sides[from].open(&f, now, &mut rng);
                }
            }
            5 => {
                now += u64::from(a) * 10_000;
                for f in sides[from].poll(now, &mut rng) {
                    wire[to].push((f.1, None));
                }
            }
            6 => {
                if let Ok(f) = sides[from].seal(lane::CTRL, &[a, b]) {
                    wire[to].push((f, Some((lane::CTRL, vec![a, b]))));
                }
            }
            _ => {
                let q = &mut wire[from];
                if !q.is_empty() {
                    q.remove(usize::from(a) % q.len());
                }
            }
        }
    }
});
