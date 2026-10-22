//! Links to nearby devices over Bluetooth LE or Wi-Fi, as the apps open them. The app moves bytes;
//! everything else happens here: framing (`PROTOCOL.md` §8), which channel or peer a link
//! carries, flow control and the limits that keep a hostile neighbour from costing much.
//!
//! A link carries two things:
//! - lane 255: `topic(32) || envelope`, signaling exactly as relays carry it (handshakes);
//! - lanes 0–3: session frames of the one peer the link was bound to by a handshake.
//!
//! Nothing on a link is trusted: envelopes and frames are sealed, and a link that keeps sending
//! garbage is closed.

use hush_core::envelope::PeerTag;
use hush_core::stream::{Fragmenter, Reassembler, SIGNAL_LANE, StreamDecoder, encode_record};

use crate::api::{ChatId, LocalKind, LocalMedium};
use crate::signal::ChannelId;

/// Nearby links open at once, at most.
pub const MAX_LINKS: usize = 8;
/// Records that fail to open before a link is closed.
pub const MAX_INVALID: u32 = 32;
/// Bytes handed to the app and not yet written, beyond which file chunks wait.
pub const MAX_QUEUED: usize = 1 << 20;
/// A link that has carried nothing authentic by then is closed.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// With every slot taken, a link silent this long gives way to a new one: a neighbour cannot
/// hold the few slots with silent connections (a real peer speaks within a second).
pub const SILENT_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// The link misbehaved and must be closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MustClose;

pub struct LocalLink {
    pub kind: LocalKind,
    pub medium: LocalMedium,
    decoder: StreamDecoder,
    fragmenter: Option<Fragmenter>,
    reassembler: Reassembler,
    invalid: u32,
    /// Bytes given to the app and not reported written yet.
    pub queued: usize,
    /// The peer whose session frames travel here.
    pub peer: Option<(ChatId, PeerTag)>,
    /// The channel we dialed this link for (none when the other side dialed).
    pub dialed_for: Option<ChannelId>,
    opened: std::time::Instant,
    /// Something authentic (an envelope that opened, or a peer's session) came over it.
    pub useful: bool,
}

impl LocalLink {
    pub fn new(kind: LocalKind, medium: LocalMedium, dialed_for: Option<ChannelId>) -> Self {
        let fragmenter = match kind {
            LocalKind::Stream => None,
            // Fragmenter refuses MTUs below 20; BLE's minimum ATT payload is 20 bytes.
            LocalKind::Packets { mtu } => Fragmenter::new(usize::try_from(mtu).unwrap_or(20).max(20)).ok(),
        };
        Self {
            kind,
            medium,
            decoder: StreamDecoder::new(),
            fragmenter,
            reassembler: Reassembler::new(),
            invalid: 0,
            queued: 0,
            peer: None,
            dialed_for,
            opened: std::time::Instant::now(),
            useful: false,
        }
    }

    /// Open for a while without carrying anything authentic.
    pub fn idle(&self) -> bool {
        self.silent_for(IDLE_TIMEOUT)
    }

    /// Nothing authentic came over it, in at least `d` since it opened.
    pub fn silent_for(&self, d: std::time::Duration) -> bool {
        !self.useful && self.peer.is_none() && self.opened.elapsed() >= d
    }

    pub fn opened(&self) -> std::time::Instant {
        self.opened
    }

    /// The writes that carry one record: one buffer for a stream, one per packet otherwise.
    pub fn encode(&mut self, lane: u8, data: &[u8]) -> Vec<Vec<u8>> {
        let out = match &mut self.fragmenter {
            None => vec![encode_record(lane, data)],
            Some(f) => f.split(lane, data),
        };
        self.queued += out.iter().map(Vec::len).sum::<usize>();
        out
    }

    pub fn envelope(&mut self, payload: &[u8]) -> Vec<Vec<u8>> {
        self.encode(SIGNAL_LANE, payload)
    }

    /// Bytes from the app: the complete records they finish. `Err` when the link must be closed
    /// (a framing error, or too much garbage).
    pub fn push(&mut self, data: &[u8]) -> Result<Vec<(u8, Vec<u8>)>, MustClose> {
        match self.kind {
            LocalKind::Stream => self.decoder.push(data).map_err(|_| MustClose),
            LocalKind::Packets { .. } => match self.reassembler.push(data) {
                Ok(r) => Ok(r.into_iter().collect()),
                // A lost packet drops its record; the next one starts afresh.
                Err(_) => {
                    self.strike()?;
                    Ok(Vec::new())
                }
            },
        }
    }

    /// Counts a record that did not open. `Err` once there were too many.
    pub fn strike(&mut self) -> Result<(), MustClose> {
        self.invalid += 1;
        if self.invalid > MAX_INVALID {
            Err(MustClose)
        } else {
            Ok(())
        }
    }

    pub fn sent(&mut self, bytes: usize) {
        self.queued = self.queued.saturating_sub(bytes);
    }

    pub fn has_room(&self) -> bool {
        self.queued < MAX_QUEUED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_and_packets_round_trip() {
        for kind in [LocalKind::Stream, LocalKind::Packets { mtu: 185 }] {
            let mut a = LocalLink::new(kind, LocalMedium::Bluetooth, None);
            let mut b = LocalLink::new(kind, LocalMedium::Bluetooth, None);
            let mut got = Vec::new();
            for w in a.envelope(&[7u8; 3000]).into_iter().chain(a.encode(1, b"frame")) {
                assert!(matches!(kind, LocalKind::Stream) || w.len() <= 185);
                got.extend(b.push(&w).unwrap());
            }
            assert_eq!(got, vec![(SIGNAL_LANE, vec![7u8; 3000]), (1, b"frame".to_vec())]);
            assert!(a.queued > 3000);
            a.sent(a.queued);
            assert_eq!(a.queued, 0);
        }
    }

    #[test]
    fn garbage_closes_the_link() {
        let mut l = LocalLink::new(LocalKind::Stream, LocalMedium::Wifi, None);
        assert!(l.push(&[0, 0xff, 0xff, 0xff, 0xff]).is_err());
        let mut p = LocalLink::new(LocalKind::Packets { mtu: 40 }, LocalMedium::Bluetooth, None);
        for _ in 0..MAX_INVALID {
            assert!(p.push(&[0, 0, 9]).is_ok());
        }
        assert!(p.push(&[0, 0, 9]).is_err());
    }

    #[test]
    fn only_silent_links_give_way() {
        let mut l = LocalLink::new(LocalKind::Stream, LocalMedium::Wifi, None);
        assert!(l.silent_for(std::time::Duration::ZERO));
        assert!(!l.silent_for(SILENT_GRACE), "a new link gets its grace");
        l.useful = true;
        assert!(!l.silent_for(std::time::Duration::ZERO));
    }
}
