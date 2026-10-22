//! Whole conversations between engines: over simulated Bluetooth and Wi-Fi links (no network at
//! all), and over a local relay with WebRTC.

mod common;

use std::sync::Arc;

use common::{Air, Events, config, relay};
use hush_app::contacts::Cmd as ContactCmd;
use hush_app::{
    App, ChatId, ChatKind, Cmd, EndReason, Event, LocalHost, LocalKind, LocalMedium, LookupState, Notice, PathKind,
};
use tokio::sync::mpsc;

struct Person {
    app: App,
    ev: Events,
    _dir: tempfile::TempDir,
}

fn person(name: &str, device: Option<Arc<common::Device>>, vault: bool, relays: Vec<String>) -> Person {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let data = vault.then(|| dir.path().join("data"));
    let mut cfg = config(name, out, data, relays);
    cfg.local = device.is_some();
    let (tx, rx) = mpsc::unbounded_channel();
    let host = device.clone().map(|d| d as Arc<dyn LocalHost>);
    let app = App::start(cfg, tx, host);
    if let Some(d) = device {
        d.attach(app.clone());
    }
    Person {
        app,
        ev: Events::new(rx),
        _dir: dir,
    }
}

async fn joined(p: &mut Person, chat: ChatId) -> (String, Vec<u8>) {
    p.ev.wait("a peer", |e| match e {
        Event::PeerJoined { chat: c, peer, sas, .. } if *c == chat => Some((peer.clone(), sas.clone())),
        _ => None,
    })
    .await
}

async fn path(p: &mut Person, chat: ChatId) -> PathKind {
    p.ev.wait("a path", |e| match e {
        Event::PeerPath { chat: c, path, .. } if *c == chat && *path != PathKind::Connecting => Some(*path),
        _ => None,
    })
    .await
}

async fn verified(p: &mut Person, chat: ChatId) {
    p.ev.wait("verified by both", |e| match e {
        Event::PeerVerified {
            chat: c,
            by_me: true,
            by_peer: true,
            ..
        } if *c == chat => Some(()),
        _ => None,
    })
    .await;
}

async fn message(p: &mut Person, chat: ChatId, text: &str) -> String {
    p.ev.wait(text, |e| match e {
        Event::Message {
            chat: c,
            peer: Some(_),
            text: t,
            author,
            ..
        } if *c == chat && t == text => Some(author.clone()),
        _ => None,
    })
    .await
}

/// Link, emojis, verification, messages both ways, a file, and the end.
async fn link_conversation(alice: &mut Person, bob: &mut Person, expect_path: impl Fn(PathKind) -> bool) {
    let (ca, url) = alice
        .app
        .ask(|reply| Cmd::CreateLink {
            group: false,
            ttl_min: 10,
            passphrase: None,
            reply,
        })
        .await
        .unwrap();
    let cb = bob
        .app
        .ask(|reply| Cmd::JoinLink {
            input: url,
            passphrase: None,
            reply,
        })
        .await
        .unwrap();
    let (bob_tag, sas_a) = joined(alice, ca).await;
    let (_, sas_b) = joined(bob, cb).await;
    assert_eq!(sas_a, sas_b, "both see the same emojis");
    assert_eq!(sas_a.len(), 5);
    assert!(expect_path(path(alice, ca).await));
    assert!(expect_path(path(bob, cb).await));

    alice.app.send(Cmd::Verify {
        chat: ca,
        peer: Some(bob_tag),
        ok: true,
    });
    bob.app.send(Cmd::Verify {
        chat: cb,
        peer: None,
        ok: true,
    });
    verified(alice, ca).await;
    verified(bob, cb).await;

    alice.app.send(Cmd::Send {
        chat: ca,
        text: "hello Bob".into(),
    });
    assert_eq!(message(bob, cb, "hello Bob").await, "Alice");
    bob.app.send(Cmd::Send {
        chat: cb,
        text: "hello Alice\nsecond line".into(),
    });
    assert_eq!(message(alice, ca, "hello Alice\nsecond line").await, "Bob");

    // A file, bigger than a chunk.
    let src = alice._dir.path().join("photo.bin");
    let data: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
    std::fs::write(&src, &data).unwrap();
    alice.app.send(Cmd::SendFile { chat: ca, path: src });
    let id = bob
        .ev
        .wait("a file offer", |e| match e {
            Event::FileOffered {
                chat,
                peer: Some(_),
                id,
                name,
                ..
            } if *chat == cb && name == "photo.bin" => Some(id.clone()),
            _ => None,
        })
        .await;
    bob.app.send(Cmd::GetFile { chat: cb, id });
    let saved = bob
        .ev
        .wait("the file saved", |e| match e {
            Event::FileSaved { chat, path, .. } if *chat == cb => Some(path.clone()),
            _ => None,
        })
        .await;
    assert_eq!(std::fs::read(saved).unwrap(), data);
    alice
        .ev
        .wait("delivery", |e| {
            matches!(e, Event::FileDelivered { chat, .. } if *chat == ca).then_some(())
        })
        .await;

    alice.app.send(Cmd::End { chat: ca });
    let reason = bob
        .ev
        .wait("the end", |e| match e {
            Event::ChatEnded { chat, reason } if *chat == cb => Some(*reason),
            _ => None,
        })
        .await;
    assert_eq!(reason, EndReason::PeerLeft);
}

#[tokio::test(flavor = "multi_thread")]
async fn over_wifi_without_any_network() {
    let air = Air::new(LocalKind::Stream);
    let mut alice = person("Alice", Some(air.device()), false, vec![]);
    let mut bob = person("Bob", Some(air.device()), false, vec![]);
    link_conversation(&mut alice, &mut bob, |p| p == PathKind::Local(LocalMedium::Wifi)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn over_bluetooth_packets_without_any_network() {
    let air = Air::new(LocalKind::Packets { mtu: 185 });
    let mut alice = person("Alice", Some(air.device()), false, vec![]);
    let mut bob = person("Bob", Some(air.device()), false, vec![]);
    link_conversation(&mut alice, &mut bob, |p| p == PathKind::Local(LocalMedium::Bluetooth)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn over_a_relay_and_webrtc() {
    let r = relay().await;
    let mut alice = person("Alice", None, false, vec![r.clone()]);
    let mut bob = person("Bob", None, false, vec![r]);
    link_conversation(&mut alice, &mut bob, |p| p == PathKind::Direct).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_emojis_end_the_conversation() {
    let air = Air::new(LocalKind::Stream);
    let mut alice = person("Alice", Some(air.device()), false, vec![]);
    let mut bob = person("Bob", Some(air.device()), false, vec![]);
    let (ca, url) = alice
        .app
        .ask(|reply| Cmd::CreateLink {
            group: false,
            ttl_min: 10,
            passphrase: None,
            reply,
        })
        .await
        .unwrap();
    let cb = bob
        .app
        .ask(|reply| Cmd::JoinLink {
            input: url,
            passphrase: None,
            reply,
        })
        .await
        .unwrap();
    joined(&mut alice, ca).await;
    joined(&mut bob, cb).await;
    bob.app.send(Cmd::Verify {
        chat: cb,
        peer: None,
        ok: false,
    });
    let r = alice
        .ev
        .wait("the end", |e| match e {
            Event::ChatEnded { chat, reason } if *chat == ca => Some(*reason),
            _ => None,
        })
        .await;
    assert_eq!(r, EndReason::PeerLeft);
}

async fn create_vault(p: &mut Person) {
    p.app
        .ask(|reply| {
            Cmd::Contacts(ContactCmd::CreateVault {
                passphrase: "a long vault passphrase".into(),
                reply,
            })
        })
        .await
        .unwrap();
}

/// Saving each other in a verified link conversation; returns (alice's chat, bob's chat).
async fn save_each_other(alice: &mut Person, bob: &mut Person) -> (ChatId, ChatId) {
    let (ca, url) = alice
        .app
        .ask(|reply| Cmd::CreateLink {
            group: false,
            ttl_min: 10,
            passphrase: None,
            reply,
        })
        .await
        .unwrap();
    let cb = bob
        .app
        .ask(|reply| Cmd::JoinLink {
            input: url,
            passphrase: None,
            reply,
        })
        .await
        .unwrap();
    let (bob_tag, _) = joined(alice, ca).await;
    let (alice_tag, _) = joined(bob, cb).await;

    // Before the emojis are compared, saving is refused.
    alice.app.send(Cmd::SaveContact {
        chat: ca,
        peer: bob_tag.clone(),
    });
    alice
        .ev
        .wait("verify first", |e| {
            matches!(
                e,
                Event::Notice {
                    notice: Notice::VerifyFirst,
                    ..
                }
            )
            .then_some(())
        })
        .await;

    alice.app.send(Cmd::Verify {
        chat: ca,
        peer: None,
        ok: true,
    });
    bob.app.send(Cmd::Verify {
        chat: cb,
        peer: None,
        ok: true,
    });
    verified(alice, ca).await;
    verified(bob, cb).await;
    alice.app.send(Cmd::SaveContact {
        chat: ca,
        peer: bob_tag,
    });
    let offer = bob
        .ev
        .wait("a card offer", |e| match e {
            Event::CardOffer { chat, name, .. } if *chat == cb => Some(name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(offer, "Alice");
    bob.app.send(Cmd::SaveContact {
        chat: cb,
        peer: alice_tag,
    });
    for p in [&mut *alice, &mut *bob] {
        p.ev.wait("contact saved", |e| {
            matches!(
                e,
                Event::Notice {
                    notice: Notice::ContactSaved { .. },
                    ..
                }
            )
            .then_some(())
        })
        .await;
    }
    (ca, cb)
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_contacts_find_each_other_nearby() {
    let air = Air::new(LocalKind::Packets { mtu: 244 });
    let (da, db) = (air.device(), air.device());
    let mut alice = person("Alice", Some(da.clone()), true, vec![]);
    let mut bob = person("Bob", Some(db), true, vec![]);
    create_vault(&mut alice).await;
    create_vault(&mut bob).await;
    let (ca, cb) = save_each_other(&mut alice, &mut bob).await;
    // Kept history, on Alice's side.
    let key = alice
        .ev
        .wait("contacts", |e| match e {
            Event::Contacts(list) if list.len() == 1 => Some(list[0].key.clone()),
            _ => None,
        })
        .await;
    alice.app.send(Cmd::Contacts(ContactCmd::SetHistory { key, on: true }));

    alice.app.send(Cmd::End { chat: ca });
    bob.ev
        .wait("the end", |e| {
            matches!(e, Event::ChatEnded { chat, .. } if *chat == cb).then_some(())
        })
        .await;
    da.drop_links();
    alice.ev.clear();
    bob.ev.clear();

    // No link, no network: they find each other as contacts over Bluetooth.
    let chat_a = alice
        .ev
        .wait("a contact chat", |e| match e {
            Event::ChatOpened {
                chat,
                kind: ChatKind::Contact { .. },
                ..
            } => Some(*chat),
            _ => None,
        })
        .await;
    let chat_b = bob
        .ev
        .wait("a contact chat", |e| match e {
            Event::ChatOpened {
                chat,
                kind: ChatKind::Contact { .. },
                ..
            } => Some(*chat),
            _ => None,
        })
        .await;
    let (_, sas) = joined(&mut alice, chat_a).await;
    assert!(sas.is_empty(), "contacts were verified when saved");
    joined(&mut bob, chat_b).await;
    assert_eq!(path(&mut alice, chat_a).await, PathKind::Local(LocalMedium::Bluetooth));
    alice.app.send(Cmd::Send {
        chat: chat_a,
        text: "found you".into(),
    });
    assert_eq!(message(&mut bob, chat_b, "found you").await, "Alice");

    // Locked vault: the history is there, sealed, after unlocking.
    alice.app.send(Cmd::Contacts(ContactCmd::LockVault));
    alice
        .app
        .ask(|reply| {
            Cmd::Contacts(ContactCmd::UnlockVault {
                passphrase: "a long vault passphrase".into(),
                reply,
            })
        })
        .await
        .unwrap();
    let backup = alice
        .app
        .ask(|reply| {
            Cmd::Contacts(ContactCmd::ExportBackup {
                passphrase: "backup pass".into(),
                reply,
            })
        })
        .await
        .unwrap();
    let plain = hush_core::backup::import("backup pass", &backup).unwrap();
    let snap: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(snap["contacts"][0]["name"], "Bob");
    let kept = snap["history"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .as_array()
        .unwrap();
    assert!(kept.iter().any(|m| m["text"] == "found you" && m["mine"] == true));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_by_identifier() {
    let r = relay().await;
    let mut alice = person("Alice", None, true, vec![r.clone()]);
    let mut bob = person("Bob", None, true, vec![r]);
    create_vault(&mut alice).await;
    create_vault(&mut bob).await;
    let id = alice
        .app
        .ask(|reply| Cmd::Contacts(ContactCmd::EnsureIdentity { reply }))
        .await
        .unwrap();
    alice.app.send(Cmd::Contacts(ContactCmd::SetReachable(true)));
    alice
        .ev
        .wait("reachable", |e| matches!(e, Event::Reachable(true)).then_some(()))
        .await;

    // A typo is refused at once.
    bob.app
        .send(Cmd::Contacts(ContactCmd::AddById { id: "hush1typo".into() }));
    bob.ev
        .wait("not found", |e| {
            matches!(
                e,
                Event::Lookup {
                    state: LookupState::NotFound,
                    ..
                }
            )
            .then_some(())
        })
        .await;

    bob.app.send(Cmd::Contacts(ContactCmd::AddById { id: id.clone() }));
    bob.ev
        .wait("requested", |e| {
            matches!(
                e,
                Event::Lookup {
                    state: LookupState::Requested,
                    ..
                }
            )
            .then_some(())
        })
        .await;
    let from = alice
        .ev
        // The requester solves a proof of work first.
        .wait_for("a request", std::time::Duration::from_secs(120), |e| match e {
            Event::Requests(list) if !list.is_empty() => Some(list[0].clone()),
            _ => None,
        })
        .await;
    assert_eq!(from.name, "Bob");
    alice.app.send(Cmd::Contacts(ContactCmd::Accept { id: from.id }));
    let ca = alice
        .ev
        .wait("a stranger chat", |e| match e {
            Event::ChatOpened {
                chat,
                kind: ChatKind::Stranger { .. },
                ..
            } => Some(*chat),
            _ => None,
        })
        .await;
    let cb = bob
        .ev
        .wait("a stranger chat", |e| match e {
            Event::ChatOpened {
                chat,
                kind: ChatKind::Stranger { .. },
                ..
            } => Some(*chat),
            _ => None,
        })
        .await;
    joined(&mut alice, ca).await;
    joined(&mut bob, cb).await;
    for p in [&mut alice, &mut bob] {
        p.ev.wait("contact saved", |e| {
            matches!(
                e,
                Event::Notice {
                    notice: Notice::ContactSaved { .. },
                    ..
                }
            )
            .then_some(())
        })
        .await;
    }
    bob.app.send(Cmd::Send {
        chat: cb,
        text: "hi, it's Bob".into(),
    });
    assert_eq!(message(&mut alice, ca, "hi, it's Bob").await, "Bob");
}
