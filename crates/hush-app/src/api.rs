//! What the apps ask of the engine, and what it tells them.

use std::path::PathBuf;

use crate::rtc::IceServer;
use crate::signal::{LinkId, SignalConfig};
use crate::store::VaultState;

/// A conversation: one link (one-to-one or a group), one saved contact, or someone met by
/// identifier.
pub type ChatId = u64;

/// How the engine is set up. Everything but `out_dir` and `data_dir` can change later.
#[derive(Clone, Debug)]
pub struct Config {
    /// Our display name, sent to peers.
    pub name: String,
    /// Base URL of printed links (`https://maxgfr.github.io/hush/`).
    pub base: String,
    pub signal: SignalConfig,
    pub ice_servers: Vec<IceServer>,
    /// Only use TURN: peers never learn our IP address.
    pub relay_only: bool,
    /// Where received files are saved.
    pub out_dir: PathBuf,
    /// Where the vault lives; `None` keeps nothing on disk.
    pub data_dir: Option<PathBuf>,
    /// Look for peers over Bluetooth and Wi-Fi (the app provides the links).
    pub local: bool,
    /// Message timer for what we send, in seconds (0 = off).
    pub ttl: u64,
    /// Delete received files when they disappear, when their sender deletes them, and when the
    /// conversation ends: the phone apps keep them where nothing else could reach them later.
    /// The CLI saves into a folder the user chose, and leaves them there.
    pub discard_files: bool,
    /// Blossom servers that keep a sealed copy of a file when no direct path carries it (empty:
    /// files need a direct path).
    pub blossom: Vec<String>,
}

/// What kind of link a nearby connection is, which decides its framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    /// A byte stream: TCP over Wi-Fi, or a Bluetooth L2CAP channel.
    Stream,
    /// Small packets of at most `mtu` bytes: Bluetooth GATT writes and notifications.
    Packets { mtu: u32 },
}

/// Which medium a nearby link uses, for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalMedium {
    Bluetooth,
    Wifi,
}

/// The native side of nearby links (Bluetooth LE, Wi-Fi). Calls arrive from the engine's thread.
pub trait LocalHost: Send + Sync {
    /// Advertise these tokens (16 bytes each; empty: stop advertising).
    fn advertise(&self, tokens: Vec<Vec<u8>>);
    /// Look for devices advertising any of these tokens (empty: stop scanning).
    fn scan(&self, tokens: Vec<Vec<u8>>);
    /// Write bytes on a link, in order.
    fn send(&self, link: LinkId, data: Vec<u8>);
    /// Close a link.
    fn close(&self, link: LinkId);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    /// No working path yet.
    Connecting,
    /// A nearby link.
    Local(LocalMedium),
    /// A direct WebRTC connection (or through TURN).
    Direct,
    /// Relayed through the signaling servers, still end-to-end encrypted.
    Relayed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatStatus {
    /// Waiting for someone to open the link (or for the contact to come online).
    Waiting,
    Connecting,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// We ended it.
    Ended,
    /// The other side ended it (one-to-one).
    PeerLeft,
    /// Someone else already opened this one-to-one link: it may have leaked.
    Taken,
    Expired,
    /// The emojis did not match.
    Mismatch,
    /// The handshake failed or timed out.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatKind {
    Link {
        group: bool,
        creator: bool,
    },
    Contact {
        key: String,
    },
    /// Someone met through an identifier, not saved (yet).
    Stranger {
        id: String,
    },
}

/// Something to say in a conversation; the apps word it in their language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Joined {
        name: String,
    },
    Left {
        name: String,
    },
    Renamed {
        old: String,
        name: String,
    },
    Locked {
        by: String,
        locked: bool,
    },
    ContactSaved {
        name: String,
    },
    /// We sent our card; waiting for theirs.
    ContactWaiting {
        name: String,
    },
    /// Saving a contact needs the emojis compared by both sides first.
    VerifyFirst,
    /// Saving a contact needs an unlocked vault.
    VaultNeeded,
    FilesNeedDirect,
    CannotSend {
        reason: String,
    },
    NotACode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactInfo {
    pub key: String,
    pub id: String,
    pub name: String,
    pub history: bool,
    pub online: bool,
    /// The open conversation with this contact, if any.
    pub chat: Option<ChatId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestInfo {
    /// Requester's identifier (`hush1…`).
    pub id: String,
    pub name: String,
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupState {
    Searching,
    Requested,
    NotFound,
    Declined,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    ChatOpened {
        chat: ChatId,
        kind: ChatKind,
        /// The link to share, for a link we created.
        url: Option<String>,
        expires_ms: Option<u64>,
    },
    ChatStatus {
        chat: ChatId,
        status: ChatStatus,
    },
    /// A hand-carried code to show (offline mode).
    Code {
        chat: ChatId,
        code: String,
    },
    PeerJoined {
        chat: ChatId,
        peer: String,
        name: String,
        /// Emoji indices to compare (empty for contacts, verified when saved).
        sas: Vec<u8>,
        verified: bool,
    },
    PeerRenamed {
        chat: ChatId,
        peer: String,
        name: String,
    },
    PeerVerified {
        chat: ChatId,
        peer: String,
        by_me: bool,
        by_peer: bool,
    },
    PeerPath {
        chat: ChatId,
        peer: String,
        path: PathKind,
    },
    PeerLeft {
        chat: ChatId,
        peer: String,
        name: String,
    },
    Locked {
        chat: ChatId,
        locked: bool,
    },
    Message {
        chat: ChatId,
        /// `None` for our own messages.
        peer: Option<String>,
        id: String,
        author: String,
        text: String,
        ts: u64,
        expires_at: Option<u64>,
        /// Restored from kept history rather than just received.
        past: bool,
    },
    MessageDeleted {
        chat: ChatId,
        id: String,
    },
    FileOffered {
        chat: ChatId,
        peer: Option<String>,
        id: String,
        name: String,
        size: u64,
        mime: String,
        expires_at: Option<u64>,
    },
    FileProgress {
        chat: ChatId,
        id: String,
        done: u64,
        total: u64,
    },
    FileSaved {
        chat: ChatId,
        id: String,
        path: PathBuf,
    },
    FileFailed {
        chat: ChatId,
        id: String,
        reason: String,
    },
    /// The receiver confirmed one of our files arrived intact.
    FileDelivered {
        chat: ChatId,
        id: String,
        peer: String,
    },
    Notice {
        chat: ChatId,
        notice: Notice,
    },
    /// A verified peer offers to save each other as contacts.
    CardOffer {
        chat: ChatId,
        peer: String,
        name: String,
        id: String,
    },
    ChatEnded {
        chat: ChatId,
        reason: EndReason,
    },
    Vault(VaultState),
    /// Our identifier, once we have one.
    Identity(Option<String>),
    Contacts(Vec<ContactInfo>),
    Requests(Vec<RequestInfo>),
    Reachable(bool),
    Lookup {
        id: String,
        state: LookupState,
        chat: Option<ChatId>,
    },
}
