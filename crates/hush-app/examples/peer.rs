//! A scriptable hush peer for interoperability tests: the same engine as the CLI and the mobile
//! apps, driven by one command per line on stdin, printing each event on stdout.
//!
//! ```text
//! peer --relay ws://… --data DIR --name Robot
//! vault <passphrase> · identity · reachable · join <link> · verify <chat> · save <chat> <peer>
//! send <chat> <text> · add <identifier> · accept <identifier> · end <chat>
//! ```

use hush_app::contacts::Cmd as ContactCmd;
use hush_app::signal::SignalConfig;
use hush_app::{App, Cmd, Config, Event};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = arg(&args, "--data").map(std::path::PathBuf::from);
    let cfg = Config {
        name: arg(&args, "--name").unwrap_or_else(|| "Robot".into()),
        base: arg(&args, "--base").unwrap_or_else(|| "https://maxgfr.github.io/hush/".into()),
        signal: SignalConfig {
            relays: arg(&args, "--relay").into_iter().collect(),
            trackers: vec![],
        },
        ice_servers: vec![],
        relay_only: false,
        out_dir: std::env::temp_dir(),
        data_dir: data,
        local: false,
        ttl: 0,
        discard_files: false,
        blossom: arg(&args, "--blossom").into_iter().collect(),
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = App::start(cfg, tx, None);
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            println!("{}", describe(&ev));
        }
    });
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut w = line.splitn(3, ' ');
        let (cmd, a, b) = (
            w.next().unwrap_or(""),
            w.next().unwrap_or("").to_string(),
            w.next().unwrap_or("").to_string(),
        );
        let chat = || a.parse().unwrap_or(0);
        match cmd {
            "vault" => {
                let r = app
                    .ask(|reply| {
                        Cmd::Contacts(ContactCmd::CreateVault {
                            passphrase: a.clone(),
                            reply,
                        })
                    })
                    .await;
                println!("vault {}", if r.is_ok() { "ok" } else { "failed" });
            }
            "identity" => match app
                .ask(|reply| Cmd::Contacts(ContactCmd::EnsureIdentity { reply }))
                .await
            {
                Ok(id) => println!("identity {id}"),
                Err(e) => println!("identity failed {e}"),
            },
            "reachable" => app.send(Cmd::Contacts(ContactCmd::SetReachable(true))),
            "join" => match app
                .ask(|reply| Cmd::JoinLink {
                    input: a.clone(),
                    passphrase: None,
                    reply,
                })
                .await
            {
                Ok(c) => println!("joined {c}"),
                Err(e) => println!("join failed {e}"),
            },
            "verify" => app.send(Cmd::Verify {
                chat: chat(),
                peer: None,
                ok: true,
            }),
            "save" => app.send(Cmd::SaveContact {
                chat: chat(),
                peer: b.clone(),
            }),
            "send" => app.send(Cmd::Send {
                chat: chat(),
                text: b.clone(),
            }),
            "add" => app.send(Cmd::Contacts(ContactCmd::AddById { id: a.clone() })),
            "accept" => app.send(Cmd::Contacts(ContactCmd::Accept { id: a.clone() })),
            "end" => app.send(Cmd::End { chat: chat() }),
            _ => println!("unknown command {cmd}"),
        }
    }
}

/// One line per event: a word, then the details (stable enough for tests to grep).
fn describe(ev: &Event) -> String {
    match ev {
        Event::PeerJoined { chat, peer, name, .. } => format!("peer-joined {chat} {peer} {name}"),
        Event::PeerVerified {
            chat, by_me, by_peer, ..
        } => format!("verified {chat} {by_me} {by_peer}"),
        Event::Message { chat, author, text, .. } => format!("message {chat} {author}: {text}"),
        Event::CardOffer { chat, peer, name, .. } => format!("card-offer {chat} {peer} {name}"),
        Event::ChatOpened { chat, kind, .. } => format!("chat {chat} {kind:?}"),
        Event::ChatEnded { chat, reason } => format!("ended {chat} {reason:?}"),
        Event::Requests(list) => format!(
            "requests {}",
            list.iter()
                .map(|r| format!("{}={}", r.id, r.name))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Event::Contacts(list) => format!(
            "contacts {}",
            list.iter()
                .map(|c| format!("{}{}", c.name, if c.online { "(online)" } else { "" }))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        other => format!("{other:?}").replace('\n', " "),
    }
}
