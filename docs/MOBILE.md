# Mobile apps: Bluetooth and Wi-Fi

A browser cannot advertise over Bluetooth, open sockets or run in the background, so hush in the browser always needs some network: Nostr relays, or a shared Wi-Fi for offline QR codes. Native apps can do more:
- find nearby devices over Bluetooth LE;
- talk over Wi-Fi Direct or the local network;
- keep listening in the background.

The protocol is ready for that. Its handshakes and sessions never depend on WebRTC: they are bytes in, bytes out.

## What is already there

| Piece | Where |
|---|---|
| The whole protocol (handshakes, ratchet, envelopes, identities, backups) | `crates/hush-core`, sans-IO |
| Framing for byte streams and small packets | `hush_core::stream` (`encode_record`, `StreamDecoder`, `Fragmenter`, `Reassembler`) |
| Kotlin and Swift bindings | `crates/hush-ffi` (UniFFI) |
| A test of a full handshake and session over 185-byte BLE packets | `crates/hush-ffi/src/lib.rs` |

Generate the bindings:

```sh
cargo build -p hush-ffi --release
cargo run -p hush-ffi --features cli --bin uniffi-bindgen -- generate \
  --library target/release/libhush_ffi.dylib --language kotlin --out-dir bindings/kotlin
cargo run -p hush-ffi --features cli --bin uniffi-bindgen -- generate \
  --library target/release/libhush_ffi.dylib --language swift --out-dir bindings/swift
```

For Android, build with `cargo ndk` for each ABI. For iOS, build a static library per target and wrap it in an XCFramework.

## Transports to add

| Transport | Discovery | Data |
|---|---|---|
| Internet | Nostr / trackers (port `web/src/signal`) | WebRTC (e.g. `webrtc-rs` or the platform's), same as the web |
| Wi-Fi LAN | mDNS service `_hush._tcp`, TXT record = first 8 bytes of today's link topic | TCP, records via `StreamDecoder` |
| Wi-Fi Direct | Android Wi-Fi P2P / Apple Multipeer | TCP stream, same records |
| Bluetooth LE | Advertise a 16-byte token `Expand(topic, "ble-adv" ‖ 10-minute slot)` | L2CAP CoC (stream) or GATT writes via `Fragmenter` / `Reassembler` at the negotiated MTU |

On a local link, the handshake envelopes travel on lane 255 of the same link: `SIGNAL_LANE`. Session frames travel on lanes 0 to 2, exactly as over WebRTC.

## A store-and-forward mesh (later)

A Bridgefy-style mesh, where phones relay for each other while in pockets, needs messages that can wait for a peer that is not there. The building blocks exist:
- envelopes are sealed;
- topics are unlinkable;
- frames are padded.

Two things would be added:
- **Mailboxes:** an asynchronous ratchet such as Signal's SPQR. Today's ratchet assumes both sides are online.
- **Flooding:** TTL-limited flooding of sealed envelopes between nearby devices.

This is deliberately out of scope for version 1.
