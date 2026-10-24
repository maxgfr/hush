//! Sealed drops through Blossom servers on localhost: sealing to a link, an identity or a session
//! key, folders, a server that is down, resuming, and deleting after a one-time download.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use hush_app::blossom::Blossom;
use hush_app::drops::{self, Progress};
use hush_core::drop::{Recipient, SEGMENT, Unlock, open_header};
use hush_core::identity::Identity;
use hush_core::link::{Link, Mode};
use hush_core::vault::KdfParams;

const CHEAP: KdfParams = KdfParams { m_kib: 64, t: 1, p: 1 };

fn quiet() -> Progress {
    Arc::new(|_, _| {})
}

fn write(dir: &Path, rel: &str, len: usize, seed: u8) -> Vec<u8> {
    let data: Vec<u8> = (0..len)
        .map(|i| (i as u8).wrapping_mul(seed).wrapping_add(seed))
        .collect();
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, &data).unwrap();
    data
}

fn now() -> u64 {
    hush_app::util::now_ms()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_through_a_link_with_a_passphrase() {
    let (a, sa) = common::blossom().await;
    let (b, sb) = common::blossom().await;
    let src = tempfile::tempdir().unwrap();
    let big = write(src.path(), "album/big.bin", 2 * SEGMENT + 123, 3);
    let small = write(src.path(), "album/sub/note.txt", 42, 5);
    write(src.path(), "album/empty", 0, 1);

    let link = Link::generate(Mode::Drop, now(), 60, true, &mut hush_app::util::rng());
    let root = link.drop_root_with(Some("open sesame"), CHEAP).unwrap();
    let sources = drops::collect(&[src.path().join("album")]).unwrap();
    let prepared = drops::prepare(
        sources,
        &[Recipient::Secret(&root)],
        None,
        now() + 3_600_000,
        true,
        &quiet(),
    )
    .unwrap();
    let http = Blossom::new();
    let locator = drops::upload_header(&prepared, &http, &[a.clone(), b.clone()])
        .await
        .unwrap();
    assert_eq!(locator.servers, vec![a.clone(), b.clone()]);
    let prepared = Arc::new(prepared);
    drops::upload_segments(Arc::clone(&prepared), &http, &locator, quiet())
        .await
        .unwrap();
    // Both servers hold the header and the three segments, and nothing in clear.
    for s in [&sa, &sb] {
        let blobs = s.blobs.lock().unwrap();
        assert_eq!(blobs.len(), 4);
        assert!(!blobs.values().any(|b| b.windows(16).any(|w| w == &small[..16])));
    }

    // The recipient parses the link, fetches the header and downloads; one server is down.
    let mut shared = link.clone();
    shared.locator = Some(locator.clone());
    let parsed = Link::parse(&shared.fragment()).unwrap();
    let loc = parsed.locator.clone().unwrap();
    sa.down.store(true, Ordering::SeqCst);
    let header = drops::fetch_header(&http, &loc).await.unwrap();
    let root2 = parsed.drop_root_with(Some("open sesame"), CHEAP).unwrap();
    assert!(
        open_header(
            &header,
            &Unlock::Secret(&parsed.drop_root_with(Some("nope"), CHEAP).unwrap()),
            now()
        )
        .is_err()
    );
    let opened = open_header(&header, &Unlock::Secret(&root2), now()).unwrap();
    assert!(opened.manifest.once);
    let out = tempfile::tempdir().unwrap();
    let saved = drops::download(&opened, &loc, &http, out.path(), quiet())
        .await
        .unwrap();
    assert_eq!(saved.len(), 3);
    assert_eq!(std::fs::read(out.path().join("album/big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(out.path().join("album/sub/note.txt")).unwrap(), small);
    assert_eq!(std::fs::read(out.path().join("album/empty")).unwrap(), Vec::<u8>::new());
    // The staging folder is gone.
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);

    // A second download does not overwrite the first.
    let again = drops::download(&opened, &loc, &http, out.path(), quiet())
        .await
        .unwrap();
    assert!(again[0].ends_with("album/big (1).bin"), "{:?}", again[0]);

    // Once downloaded, a one-time drop is deleted from the servers that are up.
    sa.down.store(false, Ordering::SeqCst);
    assert!(drops::delete(&opened, &loc, &http).await);
    assert!(sa.blobs.lock().unwrap().is_empty() && sb.blobs.lock().unwrap().is_empty());
    assert!(drops::fetch_header(&http, &loc).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn sealed_to_identities_and_signed() {
    let (a, _) = common::blossom().await;
    let src = tempfile::tempdir().unwrap();
    let data = write(src.path(), "contract.pdf", 5000, 7);
    let mut rng = hush_app::util::rng();
    let alice = Identity::generate(&mut rng);
    let bob = Identity::generate(&mut rng);
    let epoch = Identity::spk_epoch(hush_core::utc_day(now()));
    let (bob_ek, me_ek) = (bob.spk(epoch).public(), alice.spk(epoch).public());
    let prepared = drops::prepare(
        drops::collect(&[src.path().join("contract.pdf")]).unwrap(),
        &[
            Recipient::Key { ek: &bob_ek, epoch },
            Recipient::Key { ek: &me_ek, epoch },
        ],
        Some(&alice),
        now() + 60_000,
        false,
        &quiet(),
    )
    .unwrap();
    let http = Blossom::new();
    let locator = drops::upload_header(&prepared, &http, std::slice::from_ref(&a))
        .await
        .unwrap();
    drops::upload_segments(Arc::new(prepared), &http, &locator, quiet())
        .await
        .unwrap();

    let sealed = hush_core::link::Locator::parse_sealed(&locator.sealed_url("https://x.y/hush/")).unwrap();
    let header = drops::fetch_header(&http, &sealed).await.unwrap();
    for who in [&bob, &alice] {
        let opened = open_header(&header, &Unlock::Identity(who), now()).unwrap();
        assert_eq!(opened.sender.as_ref().map(|s| s.id()), Some(alice.public().id()));
        let out = tempfile::tempdir().unwrap();
        let saved = drops::download(&opened, &sealed, &http, out.path(), quiet())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&saved[0]).unwrap(), data);
    }
    let eve = Identity::generate(&mut rng);
    assert!(open_header(&header, &Unlock::Identity(&eve), now()).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_download_resumes() {
    let (a, sa) = common::blossom().await;
    let src = tempfile::tempdir().unwrap();
    let data = write(src.path(), "video.mp4", 3 * SEGMENT + 10, 11);
    let prepared = drops::prepare(
        drops::collect(&[src.path().join("video.mp4")]).unwrap(),
        &[],
        None,
        now() + 60_000,
        false,
        &quiet(),
    )
    .unwrap();
    let key = *prepared.key();
    let http = Blossom::new();
    let locator = drops::upload_header(&prepared, &http, std::slice::from_ref(&a))
        .await
        .unwrap();
    let segments = prepared.manifest.segments.clone();
    drops::upload_segments(Arc::new(prepared), &http, &locator, quiet())
        .await
        .unwrap();

    // The third segment went missing: the download stops with the first two on disk.
    let third = hex::encode(segments[2]);
    let kept = sa.blobs.lock().unwrap().remove(&third).unwrap();
    let header = drops::fetch_header(&http, &locator).await.unwrap();
    let opened = open_header(&header, &Unlock::Key(&key), now()).unwrap();
    let out = tempfile::tempdir().unwrap();
    assert!(
        drops::download(&opened, &locator, &http, out.path(), quiet())
            .await
            .is_err()
    );
    assert!(!out.path().join("video.mp4").exists());

    // It comes back: only the last two segments are fetched again.
    sa.blobs.lock().unwrap().insert(third, kept);
    let started = Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
    let s = Arc::clone(&started);
    let progress: Progress = Arc::new(move |done, _| {
        let _ = s.compare_exchange(u64::MAX, done, Ordering::SeqCst, Ordering::SeqCst);
    });
    let saved = drops::download(&opened, &locator, &http, out.path(), progress)
        .await
        .unwrap();
    assert_eq!(started.load(Ordering::SeqCst), 2 * SEGMENT as u64);
    assert_eq!(std::fs::read(&saved[0]).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_changed_after_sealing_is_not_uploaded() {
    let (a, sa) = common::blossom().await;
    let src = tempfile::tempdir().unwrap();
    write(src.path(), "f.bin", SEGMENT + 1, 2);
    let prepared = drops::prepare(
        drops::collect(&[src.path().join("f.bin")]).unwrap(),
        &[],
        None,
        now() + 60_000,
        false,
        &quiet(),
    )
    .unwrap();
    let http = Blossom::new();
    let locator = drops::upload_header(&prepared, &http, std::slice::from_ref(&a))
        .await
        .unwrap();
    // Same size, other bytes: sealing again would reuse the nonces on different plaintext.
    write(src.path(), "f.bin", SEGMENT + 1, 9);
    assert!(
        drops::upload_segments(Arc::new(prepared), &http, &locator, quiet())
            .await
            .is_err()
    );
    assert_eq!(sa.blobs.lock().unwrap().len(), 1, "only the header");
}
