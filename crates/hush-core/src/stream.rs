//! Carrying session frames over links other than WebRTC — what the mobile apps will use for
//! Bluetooth LE and Wi-Fi (Direct or LAN). WebRTC gives three message channels; these links give
//! either a byte stream (TCP, BLE L2CAP) or small packets (BLE GATT writes, 20–512 bytes).
//!
//! ```text
//! record  = lane(1) || len(u32 BE) || frame                 for byte streams
//! packet  = flags(1: bit0 first, bit1 last) || seq(u16 BE) || bytes of one record   for small MTUs
//! ```
//! Records only ever carry sealed session frames (or sealed envelopes on lane 255 for the
//! handshake), so a link adds no trust: tampering is caught by the session's AEAD.

use crate::{Error, Result};

/// Lane used to carry signaling envelopes (the handshake) over a local link.
pub const SIGNAL_LANE: u8 = 255;
/// Largest record accepted (a session frame of `MAX_PLAINTEXT` plus overhead).
pub const MAX_RECORD: usize = crate::session::MAX_PLAINTEXT + 1024;

const FIRST: u8 = 1;
const LAST: u8 = 2;

/// Encodes one record for a byte stream.
#[must_use]
pub fn encode_record(lane: u8, frame: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + frame.len());
    out.push(lane);
    out.extend_from_slice(&u32::try_from(frame.len()).expect("frame under 4 GiB").to_be_bytes());
    out.extend_from_slice(frame);
    out
}

/// Accumulates bytes from a stream and yields whole records.
#[derive(Debug, Default)]
pub struct StreamDecoder {
    buf: Vec<u8>,
}

impl StreamDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds bytes; returns every complete `(lane, frame)`.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a length exceeds [`MAX_RECORD`]: the stream must be closed.
    pub fn push(&mut self, data: &[u8]) -> Result<Vec<(u8, Vec<u8>)>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 5 {
                break;
            }
            let len = u32::from_be_bytes([self.buf[1], self.buf[2], self.buf[3], self.buf[4]]) as usize;
            if len > MAX_RECORD {
                self.buf.clear();
                return Err(Error::Malformed);
            }
            if self.buf.len() < 5 + len {
                break;
            }
            let lane = self.buf[0];
            let frame = self.buf[5..5 + len].to_vec();
            self.buf.drain(..5 + len);
            out.push((lane, frame));
        }
        Ok(out)
    }
}

/// Splits records into packets of at most `mtu` bytes.
#[derive(Debug)]
pub struct Fragmenter {
    mtu: usize,
    seq: u16,
}

impl Fragmenter {
    /// # Errors
    /// [`Error::InvalidParameter`] for an MTU below 20 bytes (the BLE minimum is 23).
    pub fn new(mtu: usize) -> Result<Self> {
        if mtu < 20 {
            return Err(Error::InvalidParameter);
        }
        Ok(Self { mtu, seq: 0 })
    }

    /// Packets for one `(lane, frame)` record.
    #[must_use]
    pub fn split(&mut self, lane: u8, frame: &[u8]) -> Vec<Vec<u8>> {
        let record = encode_record(lane, frame);
        let room = self.mtu - 3;
        let n = record.len().div_ceil(room);
        record
            .chunks(room)
            .enumerate()
            .map(|(i, chunk)| {
                let mut flags = 0;
                if i == 0 {
                    flags |= FIRST;
                }
                if i + 1 == n {
                    flags |= LAST;
                }
                let mut p = Vec::with_capacity(3 + chunk.len());
                p.push(flags);
                p.extend_from_slice(&self.seq.to_be_bytes());
                p.extend_from_slice(chunk);
                self.seq = self.seq.wrapping_add(1);
                p
            })
            .collect()
    }
}

/// Reassembles packets (in order, as BLE delivers them) into records.
#[derive(Debug, Default)]
pub struct Reassembler {
    buf: Vec<u8>,
    expect: Option<u16>,
    active: bool,
}

impl Reassembler {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one packet; returns a record when one completes.
    ///
    /// # Errors
    /// [`Error::Malformed`] for a lost or reordered packet or an oversize record. The partial
    /// record is dropped; the next `first` packet starts afresh.
    pub fn push(&mut self, packet: &[u8]) -> Result<Option<(u8, Vec<u8>)>> {
        if packet.len() < 3 {
            return Err(Error::Malformed);
        }
        let flags = packet[0];
        let seq = u16::from_be_bytes([packet[1], packet[2]]);
        if flags & FIRST != 0 {
            self.buf.clear();
            self.active = true;
        } else if !self.active || self.expect != Some(seq) {
            self.reset();
            return Err(Error::Malformed);
        }
        self.expect = Some(seq.wrapping_add(1));
        self.buf.extend_from_slice(&packet[3..]);
        if self.buf.len() > MAX_RECORD + 5 {
            self.reset();
            return Err(Error::Malformed);
        }
        if flags & LAST == 0 {
            return Ok(None);
        }
        self.active = false;
        let mut dec = StreamDecoder::new();
        let mut records = dec.push(&self.buf)?;
        self.buf.clear();
        if records.len() != 1 || !dec.buf.is_empty() {
            return Err(Error::Malformed);
        }
        Ok(records.pop())
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.active = false;
        self.expect = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_records_survive_any_split() {
        let frames: Vec<Vec<u8>> = (0..5).map(|i| vec![i as u8; 1000 * i + 3]).collect();
        let mut wire = Vec::new();
        for (i, f) in frames.iter().enumerate() {
            wire.extend_from_slice(&encode_record(i as u8, f));
        }
        for step in [1, 7, 512, wire.len()] {
            let mut dec = StreamDecoder::new();
            let mut got = Vec::new();
            for chunk in wire.chunks(step) {
                got.extend(dec.push(chunk).unwrap());
            }
            assert_eq!(got.len(), 5);
            for (i, (lane, f)) in got.iter().enumerate() {
                assert_eq!(*lane as usize, i);
                assert_eq!(f, &frames[i]);
            }
        }
        let mut dec = StreamDecoder::new();
        assert!(dec.push(&[0, 0xff, 0xff, 0xff, 0xff]).is_err());
    }

    #[test]
    fn ble_sized_packets_round_trip() {
        let mut frag = Fragmenter::new(185).unwrap();
        let mut re = Reassembler::new();
        let frame: Vec<u8> = (0..64_000u32).map(|i| (i * 13) as u8).collect();
        let packets = frag.split(2, &frame);
        assert!(packets.iter().all(|p| p.len() <= 185));
        let mut out = None;
        for p in &packets {
            if let Some(r) = re.push(p).unwrap() {
                out = Some(r);
            }
        }
        assert_eq!(out, Some((2, frame)));
        // A small frame fits in one packet.
        let one = frag.split(1, b"hi");
        assert_eq!(one.len(), 1);
        assert_eq!(re.push(&one[0]).unwrap(), Some((1, b"hi".to_vec())));
        assert!(Fragmenter::new(10).is_err());
    }

    #[test]
    fn lost_packets_are_detected() {
        let mut frag = Fragmenter::new(64).unwrap();
        let mut re = Reassembler::new();
        let packets = frag.split(0, &[7u8; 500]);
        assert!(re.push(&packets[0]).unwrap().is_none());
        assert!(re.push(&packets[2]).is_err(), "packet 1 went missing");
        // The next record starts cleanly.
        let next = frag.split(0, b"ok");
        assert_eq!(re.push(&next[0]).unwrap(), Some((0, b"ok".to_vec())));
        assert!(re.push(&[1, 0]).is_err());
    }
}
