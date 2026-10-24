//! The hush application layer, shared by the CLI and the mobile apps: conversations from links,
//! saved contacts and identifiers, the vault, and every transport (Nostr relays, WebTorrent
//! trackers, WebRTC, and the Bluetooth or Wi-Fi links the apps open). The protocol itself is in
//! `hush-core`; this crate is what `web/src/app` is to the browser.

pub mod api;
pub mod blossom;
pub mod contacts;
pub mod drops;
pub mod engine;
pub mod local;
pub mod proto;
pub mod rtc;
pub mod signal;
pub mod store;
pub mod text;
pub mod util;

pub use api::*;
pub use engine::{App, Cmd, LocalIn};
