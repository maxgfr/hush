//! The whole app engine (`hush-app`) for the phones: one `HushApp` object, events through
//! `AppListener`, and the Bluetooth and Wi-Fi links the app opens through `LocalTransport`.
//!
//! Calls that answer (`create_link`, `unlock_vault`…) block the calling thread until the engine
//! replies: call them off the UI thread (some take a second, for Argon2id).

use std::path::PathBuf;
use std::sync::Arc;

use hush_app::contacts::Cmd as ContactCmd;
use hush_app::rtc::IceServer;
use hush_app::signal::{self, SignalConfig};
use hush_app::store::VaultState;
use hush_app::{
    App, ChatKind, ChatStatus, Cmd, Config, EndReason, Event, LocalHost, LocalIn, LocalKind, LocalMedium, LookupState,
    Notice, PathKind,
};

use crate::HushError;

type Res<T> = Result<T, HushError>;

fn app_err(e: anyhow::Error) -> HushError {
    HushError::App { reason: e.to_string() }
}

// ── configuration ───────────────────────────────────────────────────────────

#[derive(uniffi::Record, Clone)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
}

#[derive(uniffi::Record, Clone)]
pub struct AppConfig {
    /// Our display name.
    pub name: String,
    /// Base of the links we create (`https://maxgfr.github.io/hush/`).
    pub base: String,
    pub relays: Vec<String>,
    pub trackers: Vec<String>,
    pub ice_servers: Vec<IceServerConfig>,
    /// Only use TURN: peers never learn our IP address.
    pub relay_only: bool,
    /// Where received files are saved.
    pub out_dir: String,
    /// Where the vault lives (`None`: nothing is ever written).
    pub data_dir: Option<String>,
    /// Look for people over Bluetooth and Wi-Fi.
    pub local: bool,
    /// Disappearing-message timer for what we send, in seconds (0 = off).
    pub ttl: u64,
}

fn ice(servers: Vec<IceServerConfig>) -> Vec<IceServer> {
    servers
        .into_iter()
        .map(|s| IceServer {
            urls: s.urls,
            username: s.username,
            credential: s.credential,
        })
        .collect()
}

/// The web app's Nostr relays.
#[uniffi::export]
pub fn default_relays() -> Vec<String> {
    signal::DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
}

/// The web app's WebTorrent trackers.
#[uniffi::export]
pub fn default_trackers() -> Vec<String> {
    signal::DEFAULT_TRACKERS.iter().map(|s| s.to_string()).collect()
}

/// The web app's STUN servers.
#[uniffi::export]
pub fn default_ice_servers() -> Vec<IceServerConfig> {
    vec![IceServerConfig {
        urls: vec![
            "stun:stun.cloudflare.com:3478".into(),
            "stun:stun.l.google.com:19302".into(),
        ],
        username: String::new(),
        credential: String::new(),
    }]
}

/// What a link says about itself, before joining (to ask for its passphrase).
#[derive(uniffi::Record)]
pub struct LinkInfo {
    pub group: bool,
    pub passphrase: bool,
    pub expires_ms: u64,
}

/// Reads a link (a full URL, a fragment, or a scanned QR code). Fails with `Outdated` for an
/// old link, `Malformed` for anything else.
#[uniffi::export]
pub fn link_info(input: String) -> Res<LinkInfo> {
    let link = hush_core::link::Link::parse(&input)?;
    Ok(LinkInfo {
        group: link.mode == hush_core::link::Mode::Group,
        passphrase: link.passphrase,
        expires_ms: link.expires_min * 60_000,
    })
}

/// Whether a `hush1…` identifier is well formed (its checksum catches typos).
#[uniffi::export]
pub fn is_identifier(input: String) -> bool {
    hush_core::id::decode(&input).is_ok()
}

// ── what the app hears ──────────────────────────────────────────────────────

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Medium {
    Bluetooth,
    Wifi,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerPath {
    Connecting,
    Bluetooth,
    Wifi,
    Direct,
    Relayed,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Waiting,
    Connecting,
    Open,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Ended,
    PeerLeft,
    Taken,
    Expired,
    Mismatch,
    Failed,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum ChatType {
    Link { group: bool, creator: bool },
    Contact { key: String },
    Stranger { id: String },
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Joined { name: String },
    Left { name: String },
    Renamed { old: String, name: String },
    Locked { by: String, locked: bool },
    ContactSaved { name: String },
    ContactWaiting { name: String },
    VerifyFirst,
    VaultNeeded,
    FilesNeedDirect,
    CannotSend { reason: String },
    NotACode,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultStatus {
    None,
    Locked,
    Unlocked,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupStatus {
    Searching,
    Requested,
    NotFound,
    Declined,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct ContactEntry {
    pub key: String,
    pub id: String,
    pub name: String,
    pub history: bool,
    pub online: bool,
    pub chat: Option<u64>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct RequestEntry {
    pub id: String,
    pub name: String,
    pub note: String,
}

/// Everything the engine tells the app (see `hush_app::Event`).
#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum AppEvent {
    ChatOpened {
        chat: u64,
        kind: ChatType,
        url: Option<String>,
        expires_ms: Option<u64>,
    },
    ChatStatus {
        chat: u64,
        status: Status,
    },
    Code {
        chat: u64,
        code: String,
    },
    PeerJoined {
        chat: u64,
        peer: String,
        name: String,
        sas: Vec<u8>,
        verified: bool,
    },
    PeerRenamed {
        chat: u64,
        peer: String,
        name: String,
    },
    PeerVerified {
        chat: u64,
        peer: String,
        by_me: bool,
        by_peer: bool,
    },
    PeerPath {
        chat: u64,
        peer: String,
        path: PeerPath,
    },
    PeerLeft {
        chat: u64,
        peer: String,
        name: String,
    },
    Locked {
        chat: u64,
        locked: bool,
    },
    Message {
        chat: u64,
        peer: Option<String>,
        id: String,
        author: String,
        text: String,
        ts: u64,
        expires_at: Option<u64>,
        past: bool,
    },
    MessageDeleted {
        chat: u64,
        id: String,
    },
    FileOffered {
        chat: u64,
        peer: Option<String>,
        id: String,
        name: String,
        size: u64,
        mime: String,
        expires_at: Option<u64>,
    },
    FileProgress {
        chat: u64,
        id: String,
        done: u64,
        total: u64,
    },
    FileSaved {
        chat: u64,
        id: String,
        path: String,
    },
    FileFailed {
        chat: u64,
        id: String,
        reason: String,
    },
    FileDelivered {
        chat: u64,
        id: String,
        peer: String,
    },
    Notice {
        chat: u64,
        notice: NoticeKind,
    },
    CardOffer {
        chat: u64,
        peer: String,
        name: String,
        id: String,
    },
    ChatEnded {
        chat: u64,
        reason: Ending,
    },
    Vault {
        status: VaultStatus,
    },
    Identity {
        id: Option<String>,
    },
    Contacts {
        list: Vec<ContactEntry>,
    },
    Requests {
        list: Vec<RequestEntry>,
    },
    Reachable {
        on: bool,
    },
    Lookup {
        id: String,
        status: LookupStatus,
        chat: Option<u64>,
    },
}

fn path(p: PathKind) -> PeerPath {
    match p {
        PathKind::Connecting => PeerPath::Connecting,
        PathKind::Local(LocalMedium::Bluetooth) => PeerPath::Bluetooth,
        PathKind::Local(LocalMedium::Wifi) => PeerPath::Wifi,
        PathKind::Direct => PeerPath::Direct,
        PathKind::Relayed => PeerPath::Relayed,
    }
}

fn notice(n: Notice) -> NoticeKind {
    match n {
        Notice::Joined { name } => NoticeKind::Joined { name },
        Notice::Left { name } => NoticeKind::Left { name },
        Notice::Renamed { old, name } => NoticeKind::Renamed { old, name },
        Notice::Locked { by, locked } => NoticeKind::Locked { by, locked },
        Notice::ContactSaved { name } => NoticeKind::ContactSaved { name },
        Notice::ContactWaiting { name } => NoticeKind::ContactWaiting { name },
        Notice::VerifyFirst => NoticeKind::VerifyFirst,
        Notice::VaultNeeded => NoticeKind::VaultNeeded,
        Notice::FilesNeedDirect => NoticeKind::FilesNeedDirect,
        Notice::CannotSend { reason } => NoticeKind::CannotSend { reason },
        Notice::NotACode => NoticeKind::NotACode,
    }
}

impl From<Event> for AppEvent {
    fn from(e: Event) -> Self {
        match e {
            Event::ChatOpened {
                chat,
                kind,
                url,
                expires_ms,
            } => AppEvent::ChatOpened {
                chat,
                kind: match kind {
                    ChatKind::Link { group, creator } => ChatType::Link { group, creator },
                    ChatKind::Contact { key } => ChatType::Contact { key },
                    ChatKind::Stranger { id } => ChatType::Stranger { id },
                },
                url,
                expires_ms,
            },
            Event::ChatStatus { chat, status } => AppEvent::ChatStatus {
                chat,
                status: match status {
                    ChatStatus::Waiting => Status::Waiting,
                    ChatStatus::Connecting => Status::Connecting,
                    ChatStatus::Open => Status::Open,
                },
            },
            Event::Code { chat, code } => AppEvent::Code { chat, code },
            Event::PeerJoined {
                chat,
                peer,
                name,
                sas,
                verified,
            } => AppEvent::PeerJoined {
                chat,
                peer,
                name,
                sas,
                verified,
            },
            Event::PeerRenamed { chat, peer, name } => AppEvent::PeerRenamed { chat, peer, name },
            Event::PeerVerified {
                chat,
                peer,
                by_me,
                by_peer,
            } => AppEvent::PeerVerified {
                chat,
                peer,
                by_me,
                by_peer,
            },
            Event::PeerPath { chat, peer, path: p } => AppEvent::PeerPath {
                chat,
                peer,
                path: path(p),
            },
            Event::PeerLeft { chat, peer, name } => AppEvent::PeerLeft { chat, peer, name },
            Event::Locked { chat, locked } => AppEvent::Locked { chat, locked },
            Event::Message {
                chat,
                peer,
                id,
                author,
                text,
                ts,
                expires_at,
                past,
            } => AppEvent::Message {
                chat,
                peer,
                id,
                author,
                text,
                ts,
                expires_at,
                past,
            },
            Event::MessageDeleted { chat, id } => AppEvent::MessageDeleted { chat, id },
            Event::FileOffered {
                chat,
                peer,
                id,
                name,
                size,
                mime,
                expires_at,
            } => AppEvent::FileOffered {
                chat,
                peer,
                id,
                name,
                size,
                mime,
                expires_at,
            },
            Event::FileProgress { chat, id, done, total } => AppEvent::FileProgress { chat, id, done, total },
            Event::FileSaved { chat, id, path: p } => AppEvent::FileSaved {
                chat,
                id,
                path: p.to_string_lossy().into_owned(),
            },
            Event::FileFailed { chat, id, reason } => AppEvent::FileFailed { chat, id, reason },
            Event::FileDelivered { chat, id, peer } => AppEvent::FileDelivered { chat, id, peer },
            Event::Notice { chat, notice: n } => AppEvent::Notice {
                chat,
                notice: notice(n),
            },
            Event::CardOffer { chat, peer, name, id } => AppEvent::CardOffer { chat, peer, name, id },
            Event::ChatEnded { chat, reason } => AppEvent::ChatEnded {
                chat,
                reason: match reason {
                    EndReason::Ended => Ending::Ended,
                    EndReason::PeerLeft => Ending::PeerLeft,
                    EndReason::Taken => Ending::Taken,
                    EndReason::Expired => Ending::Expired,
                    EndReason::Mismatch => Ending::Mismatch,
                    EndReason::Failed => Ending::Failed,
                },
            },
            Event::Vault(v) => AppEvent::Vault {
                status: match v {
                    VaultState::None => VaultStatus::None,
                    VaultState::Locked => VaultStatus::Locked,
                    VaultState::Unlocked => VaultStatus::Unlocked,
                },
            },
            Event::Identity(id) => AppEvent::Identity { id },
            Event::Contacts(list) => AppEvent::Contacts {
                list: list
                    .into_iter()
                    .map(|c| ContactEntry {
                        key: c.key,
                        id: c.id,
                        name: c.name,
                        history: c.history,
                        online: c.online,
                        chat: c.chat,
                    })
                    .collect(),
            },
            Event::Requests(list) => AppEvent::Requests {
                list: list
                    .into_iter()
                    .map(|r| RequestEntry {
                        id: r.id,
                        name: r.name,
                        note: r.note,
                    })
                    .collect(),
            },
            Event::Reachable(on) => AppEvent::Reachable { on },
            Event::Lookup { id, state, chat } => AppEvent::Lookup {
                id,
                status: match state {
                    LookupState::Searching => LookupStatus::Searching,
                    LookupState::Requested => LookupStatus::Requested,
                    LookupState::NotFound => LookupStatus::NotFound,
                    LookupState::Declined => LookupStatus::Declined,
                },
                chat,
            },
        }
    }
}

/// Receives every event, on a background thread of the engine: hop to the UI thread to show it.
#[uniffi::export(foreign)]
pub trait AppListener: Send + Sync {
    fn on_event(&self, event: AppEvent);
}

/// The app's Bluetooth and Wi-Fi radios (`PROTOCOL.md` §8). Calls come from a background thread
/// and must not block.
#[uniffi::export(foreign)]
pub trait LocalTransport: Send + Sync {
    /// Advertise these 16-byte tokens (none: stop advertising).
    fn advertise(&self, tokens: Vec<Vec<u8>>);
    /// Look for devices advertising any of these tokens (none: stop scanning).
    fn scan(&self, tokens: Vec<Vec<u8>>);
    /// Write bytes on a link, in order; report them with `HushApp::local_sent` once written.
    fn send(&self, link: u64, data: Vec<u8>);
    fn close(&self, link: u64);
}

struct Host(Arc<dyn LocalTransport>);

impl LocalHost for Host {
    fn advertise(&self, tokens: Vec<Vec<u8>>) {
        self.0.advertise(tokens);
    }
    fn scan(&self, tokens: Vec<Vec<u8>>) {
        self.0.scan(tokens);
    }
    fn send(&self, link: u64, data: Vec<u8>) {
        self.0.send(link, data);
    }
    fn close(&self, link: u64) {
        self.0.close(link);
    }
}

/// The kind of a nearby link, which decides its framing.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// A byte stream: TCP over Wi-Fi, or a Bluetooth L2CAP channel.
    Stream,
    /// Bluetooth GATT packets of at most `mtu` bytes.
    Packets { mtu: u32 },
}

// ── the engine ──────────────────────────────────────────────────────────────

#[derive(uniffi::Object)]
pub struct HushApp {
    rt: tokio::runtime::Runtime,
    app: App,
}

impl HushApp {
    fn ask<T: Send + 'static>(&self, make: impl FnOnce(hush_app::engine::Reply<T>) -> Cmd) -> Res<T> {
        self.rt.block_on(self.app.ask(make)).map_err(app_err)
    }
}

#[uniffi::export]
impl HushApp {
    /// Starts the engine. `local` provides Bluetooth and Wi-Fi, when the app has them.
    #[uniffi::constructor]
    pub fn new(
        config: AppConfig,
        listener: Arc<dyn AppListener>,
        local: Option<Arc<dyn LocalTransport>>,
    ) -> Res<Arc<Self>> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("hush")
            .enable_all()
            .build()
            .map_err(|e| HushError::App { reason: e.to_string() })?;
        let cfg = Config {
            name: hush_app::text::clean_name(&config.name),
            base: config.base,
            signal: SignalConfig {
                relays: config.relays,
                trackers: config.trackers,
            },
            ice_servers: ice(config.ice_servers),
            relay_only: config.relay_only,
            out_dir: PathBuf::from(config.out_dir),
            data_dir: config.data_dir.map(PathBuf::from),
            local: config.local,
            ttl: config.ttl,
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
        let host = local.map(|l| Arc::new(Host(l)) as Arc<dyn LocalHost>);
        let app = {
            let _guard = rt.enter();
            App::start(cfg, tx, host)
        };
        rt.spawn(async move {
            while let Some(ev) = rx.recv().await {
                listener.on_event(ev.into());
            }
        });
        Ok(Arc::new(Self { rt, app }))
    }

    // conversations

    /// A new link; returns `(chat, url)` through `ChatOpened` too.
    pub fn create_link(&self, group: bool, ttl_min: u64, passphrase: Option<String>) -> Res<u64> {
        self.ask(|reply| Cmd::CreateLink {
            group,
            ttl_min,
            passphrase,
            reply,
        })
        .map(|(chat, _)| chat)
    }

    /// Opens someone's link (slow with a passphrase: Argon2id).
    pub fn join_link(&self, input: String, passphrase: Option<String>) -> Res<u64> {
        self.ask(|reply| Cmd::JoinLink {
            input,
            passphrase,
            reply,
        })
    }

    pub fn send_text(&self, chat: u64, text: String) {
        self.app.send(Cmd::Send { chat, text });
    }

    /// Offers a file (a path the engine can read: copy picked files to the app's cache first).
    pub fn send_file(&self, chat: u64, path: String) {
        self.app.send(Cmd::SendFile {
            chat,
            path: PathBuf::from(path),
        });
    }

    pub fn get_file(&self, chat: u64, id: String) {
        self.app.send(Cmd::GetFile { chat, id });
    }

    pub fn cancel_file(&self, chat: u64, id: String) {
        self.app.send(Cmd::CancelFile { chat, id });
    }

    /// Deletes one of our messages on every side.
    pub fn delete_message(&self, chat: u64, id: String) {
        self.app.send(Cmd::Delete { chat, id });
    }

    /// The emojis match (`ok`) or not, for one peer or every peer of the chat.
    pub fn verify(&self, chat: u64, peer: Option<String>, ok: bool) {
        self.app.send(Cmd::Verify { chat, peer, ok });
    }

    pub fn lock(&self, chat: u64, locked: bool) {
        self.app.send(Cmd::Lock { chat, locked });
    }

    pub fn end(&self, chat: u64) {
        self.app.send(Cmd::End { chat });
    }

    /// A hand-carried code (`hushcode:…`, from a QR code).
    pub fn manual_code(&self, code: String) {
        self.app.send(Cmd::Manual { code });
    }

    /// Shows handshake messages as codes too (offline mode).
    pub fn set_manual(&self, on: bool) {
        self.app.send(Cmd::SetManual(on));
    }

    pub fn save_contact(&self, chat: u64, peer: String) {
        self.app.send(Cmd::SaveContact { chat, peer });
    }

    // settings

    pub fn set_name(&self, name: String) {
        self.app.send(Cmd::SetName(name));
    }

    pub fn set_ttl(&self, seconds: u64) {
        self.app.send(Cmd::SetTtl(seconds));
    }

    pub fn set_local(&self, on: bool) {
        self.app.send(Cmd::SetLocal(on));
    }

    pub fn set_network(
        &self,
        relays: Vec<String>,
        trackers: Vec<String>,
        ice_servers: Vec<IceServerConfig>,
        relay_only: bool,
    ) {
        self.app.send(Cmd::SetNetwork {
            signal: SignalConfig { relays, trackers },
            ice_servers: ice(ice_servers),
            relay_only,
        });
    }

    // vault, identity and contacts

    pub fn create_vault(&self, passphrase: String) -> Res<()> {
        self.ask(|reply| Cmd::Contacts(ContactCmd::CreateVault { passphrase, reply }))
    }

    /// Fails with "Decrypt" for a wrong passphrase.
    pub fn unlock_vault(&self, passphrase: String) -> Res<()> {
        self.ask(|reply| Cmd::Contacts(ContactCmd::UnlockVault { passphrase, reply }))
    }

    pub fn lock_vault(&self) {
        self.app.send(Cmd::Contacts(ContactCmd::LockVault));
    }

    pub fn erase_vault(&self) -> Res<()> {
        self.ask(|reply| Cmd::Contacts(ContactCmd::EraseVault { reply }))
    }

    pub fn change_passphrase(&self, passphrase: String) -> Res<()> {
        self.ask(|reply| Cmd::Contacts(ContactCmd::ChangePassphrase { passphrase, reply }))
    }

    pub fn export_backup(&self, passphrase: String) -> Res<Vec<u8>> {
        self.ask(|reply| Cmd::Contacts(ContactCmd::ExportBackup { passphrase, reply }))
    }

    /// Returns how many contacts the backup restored.
    pub fn import_backup(&self, file: Vec<u8>, passphrase: String) -> Res<u64> {
        self.ask(|reply| {
            Cmd::Contacts(ContactCmd::ImportBackup {
                file,
                passphrase,
                reply,
            })
        })
        .map(|n| n as u64)
    }

    /// Our `hush1…` identifier, created if needed (needs an unlocked vault).
    pub fn ensure_identity(&self) -> Res<String> {
        self.ask(|reply| Cmd::Contacts(ContactCmd::EnsureIdentity { reply }))
    }

    pub fn remove_contact(&self, key: String) {
        self.app.send(Cmd::Contacts(ContactCmd::Remove { key }));
    }

    pub fn set_history(&self, key: String, on: bool) {
        self.app.send(Cmd::Contacts(ContactCmd::SetHistory { key, on }));
    }

    pub fn rename_contact(&self, key: String, name: String) {
        self.app.send(Cmd::Contacts(ContactCmd::Rename { key, name }));
    }

    /// The live conversation with a contact, if any (and says we are here, so it connects).
    pub fn chat_with(&self, key: String) -> Res<Option<u64>> {
        self.ask(|reply| Cmd::Contacts(ContactCmd::ChatWith { key, reply }))
    }

    pub fn set_reachable(&self, on: bool) {
        self.app.send(Cmd::Contacts(ContactCmd::SetReachable(on)));
    }

    pub fn add_by_id(&self, id: String) {
        self.app.send(Cmd::Contacts(ContactCmd::AddById { id }));
    }

    pub fn accept_request(&self, id: String) {
        self.app.send(Cmd::Contacts(ContactCmd::Accept { id }));
    }

    pub fn decline_request(&self, id: String) {
        self.app.send(Cmd::Contacts(ContactCmd::Decline { id }));
    }

    // nearby links, as the radios report them

    /// A link opened. `token`: the one the device was found by, when we connected to it.
    pub fn local_opened(&self, link: u64, kind: LinkKind, medium: Medium, token: Option<Vec<u8>>) {
        self.app.local(LocalIn::Opened {
            link,
            kind: match kind {
                LinkKind::Stream => LocalKind::Stream,
                LinkKind::Packets { mtu } => LocalKind::Packets { mtu },
            },
            medium: match medium {
                Medium::Bluetooth => LocalMedium::Bluetooth,
                Medium::Wifi => LocalMedium::Wifi,
            },
            token,
        });
    }

    pub fn local_data(&self, link: u64, data: Vec<u8>) {
        self.app.local(LocalIn::Data { link, data });
    }

    /// Bytes given through `LocalTransport::send` are written.
    pub fn local_sent(&self, link: u64, bytes: u64) {
        self.app.local(LocalIn::Sent {
            link,
            bytes: usize::try_from(bytes).unwrap_or(usize::MAX),
        });
    }

    pub fn local_closed(&self, link: u64) {
        self.app.local(LocalIn::Closed { link });
    }

    /// Ends every conversation, erases every key, and stops.
    pub fn shutdown(&self) {
        let _ = self.ask(|reply| Cmd::Shutdown { reply });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Collect(Mutex<Vec<AppEvent>>);

    impl AppListener for Collect {
        fn on_event(&self, event: AppEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn engine_starts_creates_a_link_and_stops() {
        let dir = std::env::temp_dir().join(format!("hush-ffi-{}", std::process::id()));
        let events = Arc::new(Collect::default());
        let app = HushApp::new(
            AppConfig {
                name: "Phone".into(),
                base: "https://maxgfr.github.io/hush/".into(),
                relays: vec![],
                trackers: vec![],
                ice_servers: vec![],
                relay_only: false,
                out_dir: dir.to_string_lossy().into(),
                data_dir: Some(dir.join("data").to_string_lossy().into()),
                local: false,
                ttl: 0,
            },
            events.clone(),
            None,
        )
        .unwrap();
        let chat = app.create_link(true, 30, None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let url = events
            .0
            .lock()
            .unwrap()
            .iter()
            .find_map(|e| match e {
                AppEvent::ChatOpened { chat: c, url, .. } if *c == chat => url.clone(),
                _ => None,
            })
            .expect("the link");
        let l = link_info(url).unwrap();
        assert!(l.group && !l.passphrase);
        assert!(matches!(
            link_info("https://x/#1.s.AAAA.1".into()),
            Err(HushError::Core { .. })
        ));
        assert!(app.unlock_vault("nothing yet".into()).is_err());
        app.create_vault("a long passphrase".into()).unwrap();
        assert!(app.ensure_identity().unwrap().starts_with("hush1"));
        assert!(is_identifier(app.ensure_identity().unwrap()));
        app.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }
}
