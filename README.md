# hush

**Talk without leaving a trace.** Create a link, send it, talk. Everything is end-to-end encrypted with post-quantum cryptography, there is no account and no server, and when you end the conversation every key is erased.

**→ [maxgfr.github.io/hush](https://maxgfr.github.io/hush/)** · installable as an app (PWA) · CLI: `brew install maxgfr/tap/hush`

## How it works

1. **You create a link.** It holds a random 256-bit secret in its `#fragment`, which browsers never send to any server. It expires (10 min to 7 days), a one-to-one link works only once, and it can also require a passphrase that you give another way.
2. **The other person opens it.** The two devices find each other through public Nostr relays and WebTorrent trackers, or through QR codes with no network at all. Those only ever see a topic that changes daily and ciphertext.
3. **A post-quantum handshake runs.** It uses X-Wing (X25519 + ML-KEM-768), keyed by the link, and everything after it, including your IP addresses, is encrypted. You compare 5 emojis: if they match, nobody is sitting between you.
4. **You talk.** Messages go peer to peer over WebRTC, or are relayed (still end-to-end encrypted) when no direct path exists. They travel in a ratchet:
   - a new key for every message (forward secrecy);
   - a fresh post-quantum key exchange every 5 minutes (post-compromise security);
   - every message padded so its length leaks almost nothing.
5. **You end it.** Both sides erase the keys and the messages. Nothing was ever written to disk.

| | |
|---|---|
| One-to-one links | The first person to open the link takes it. A latecomer is told "link already used", which reveals an interception. |
| Group links | A full mesh of pairwise post-quantum sessions, up to 16 people. The room can be locked. |
| Files | Up to 4 GB, files and folders, sent peer to peer and checked with SHA-256. Without a direct path they come through a sealed copy. |
| Drops | Send files by link, like a transfer service: peer to peer while you are both online, otherwise from a sealed copy on public blob servers that only ever see ciphertext. One-time or not, with a passphrase, or sealed to contacts or yourself so the link holds no key. |
| Disappearing messages | A timer from 30 s to 1 week, enforced on both sides. |
| Identifier, no sign-up | `hush1…` is computed on your device from your public keys (Ed25519 + ML-DSA-65), so nobody can impersonate it. |
| Contacts (opt-in) | Reconnect later without a new link. Saved contacts are kept in an encrypted vault (Argon2id + XChaCha20-Poly1305) that only your passphrase opens. |
| Requests by identifier (opt-in) | Someone who knows your identifier can ask to talk, with a proof of work against spam, and only once you accept. |
| Encrypted backups | One file with your identifier, contacts and any history you chose to keep. The CLI reads the same format. |
| Offline | Swap the handshake as QR codes on a shared Wi-Fi or hotspot, with no Internet at all. |

The cryptography (key exchange, signatures, ratchet, envelopes, vault, backups) is a single Rust crate. It is compiled to WebAssembly for the web app. Natively, it powers the engine shared by the CLI and the Android and iOS apps, which also talk over Bluetooth and Wi-Fi with no network at all. See [PROTOCOL.md](PROTOCOL.md) for every byte and [SECURITY.md](SECURITY.md) for what is protected and what is not.

## Cryptography

| Purpose | Algorithm | Strength |
|---|---|---|
| Key exchange | X-Wing: X25519 + ML-KEM-768 (FIPS 203) | NIST category 3. Safe as long as either half holds. |
| Signatures | Ed25519 **and** ML-DSA-65 (FIPS 204), both required | Category 3, conjunctive. |
| Encryption | ChaCha20-Poly1305 (XChaCha20-Poly1305 at rest) | 256-bit keys. |
| Key derivation | HKDF-SHA-256, Argon2id (64 MiB) for passphrases | |
| Verification | 5-emoji SAS from committed nonces | 2⁻³⁰ per attempt. |

That is much stronger than RSA, which a quantum computer breaks outright. It still is not "uncrackable": [SECURITY.md](SECURITY.md) lists the limits honestly.

## CLI

```bash
brew install maxgfr/tap/hush
hush chat                                   # prints a link (and a QR code), then chat in the terminal
hush chat 'https://maxgfr.github.io/hush/#2.s.…'
hush send report.pdf photos/                # prints a link: peer to peer now, a sealed copy for later
hush send film.mkv --p2p-only               # nothing stored: the link works while this runs
hush send contrat.pdf --to Alice --identity hush-backup.hush   # sealed to a contact, signed by you
hush receive 'https://maxgfr.github.io/hush/#2.d.…' --out ~/Downloads
hush drop rm 'https://maxgfr.github.io/hush/#2.d.…'           # delete the sealed copy
hush backup inspect hush-backup.hush        # read a backup made by the web app
hush new --group --expires 1440             # just print a link
```

In a chat:
- `/ok` and `/no` confirm or reject the emojis.
- `/file PATH` sends a file.
- `/get ID` downloads one.
- `/code CODE` pastes an offline code.
- `/quit` ends the conversation and erases the keys.

Useful flags:
- `--passphrase` (or `HUSH_PASSPHRASE`) for links that need one.
- `--ttl SECONDS` for disappearing messages.
- `--relay-only --turn URL` to hide your IP address.
- `--server HOST` (or `HUSH_BLOSSOM`) for the blob servers of sealed copies, `--no-servers` for none.
- `--many` keeps a drop after its first download; `--expires MINUTES` (10 to 10080).
- `--offline` to swap QR codes with no network.

## Phones

The Android and iOS apps open the same links, groups, contacts and backups as the web app. Nearby, they also find each other over Bluetooth LE or Wi-Fi, with no network at all. See [docs/MOBILE.md](docs/MOBILE.md) for how they work, how to build and install them, and the two-phone test checklist.

## Development

```bash
# Rust workspace: core, CLI, WebAssembly and mobile bindings
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Web app (Bun, Preact): needs the wasm-bindgen CLI matching Cargo.lock
cargo install wasm-bindgen-cli --version 0.2.128 --locked
cd web
bun install
bun run wasm           # builds the Rust core to src/core/pkg
bun run dev            # http://localhost:5173 with a local relay and tracker
bun run test           # unit tests
bun run test:e2e       # Playwright: browsers, phones, CLI and engine interop
                       # (first: cargo build -p hush-cli && cargo build -p hush-app --example peer)
```

```
crates/hush-core    the protocol: X-Wing, ML-DSA, handshakes, ratchet, envelopes, vault, backups, framing
crates/hush-wasm    WebAssembly bindings for the web app
crates/hush-app     the engine shared by the CLI and the phones: conversations, contacts, vault, every transport
crates/hush-cli     the `hush` command
crates/hush-ffi     UniFFI bindings (Kotlin, Swift) of the engine for the phones: see docs/MOBILE.md
mobile/             the Android (Compose) and iOS (SwiftUI) apps and their build scripts
web/                the web app: signaling, WebRTC, UI, service worker
```

## License

MIT
