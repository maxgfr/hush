# Security

hush is built for one thing: two (or a few) people exchanging messages and files so that nobody else can read them, now or later, and so that nothing is left once the conversation ends. This page says what that holds against, and where it stops.

Report vulnerabilities privately through [GitHub security advisories](https://github.com/maxgfr/hush/security/advisories/new).

## Keys

| Key | Where it comes from | Where it lives | Lifetime |
|---|---|---|---|
| Link secret (256 bits) | Random, in the link's `#fragment` | The link, until it expires | 10 min – 7 days |
| Link passphrase (optional) | You, through another channel | Mixed into the handshake key with Argon2id (64 MiB, 3 passes) | |
| Handshake keys | Fresh X-Wing (X25519 + ML-KEM-768) keys | Memory | One handshake |
| Session root and chains | The handshake | Memory | Rekeyed every 5 min or 1000 messages; erased on "End" |
| Identity seed (optional) | Random, on your device | The encrypted vault | Until you delete it |
| Vault key | Random, wrapped by Argon2id(passphrase) | IndexedDB (web) | Until you erase the vault |

## Layers

1. **The link.** It is a 256-bit secret in the URL fragment, which browsers never send to the web server. Every bootstrap key derives from it: the rendezvous topic (rotating daily), the envelope key and the handshake pre-shared key. With a passphrase, the handshake key also needs the passphrase. The topic does not, so a wrong passphrase fails visibly.
2. **Signaling envelopes.** Messages to the relays are sealed with XChaCha20-Poly1305 under the link key and bound to the topic. Sender, recipient, type, time and length (padded) are all inside, and stale or replayed envelopes are dropped.
3. **The handshake** (`NNpsk0 + X-Wing`, three messages).
   - The joiner's first message proves it holds the link (and passphrase).
   - The creator encapsulates to the joiner's fresh X-Wing key.
   - The WebRTC offer and answer (which carry IP addresses) only travel in messages 2 and 3, encrypted under the post-quantum handshake key.
   - Each side commits to a random nonce before seeing the other's, and the code only covers what was fixed before either nonce was revealed. An attacker in the middle therefore cannot grind the 5-emoji code (30 bits): it gets one try.
   - Every first message gets its own key (Noise's `psk` rule), even when many people open the same link.
4. **The session.**
   - **Per message:** each direction and lane has a KDF chain. Every message key is used once and forgotten, which gives forward secrecy.
   - **Every 5 minutes or 1000 messages:** a fresh X-Wing exchange moves the session to a new root and erases the old one. This is post-compromise security against quantum adversaries too.
   - **Every frame:** padded with Padmé (at least 256 bytes). Replays, reflections, cross-lane moves and forgeries are rejected without disturbing the state.
5. **Transport.** WebRTC DTLS wraps the session frames as a second layer. hush does not rely on it: the same frames are relayed through Nostr when no direct path exists, and will travel over Bluetooth or Wi-Fi in the mobile apps.
6. **At rest (opt-in).** Records in the vault are sealed with XChaCha20-Poly1305 under a key wrapped by Argon2id(passphrase), and their storage keys are HMACs so contact identifiers do not show. Backups use the same construction. Nothing is written to disk unless you create a vault.

## Identity without sign-up

An identifier is `hush1` + bech32m(SHA-256(Ed25519 public key ‖ ML-DSA-65 public key)).
- Whoever presents keys that hash to it is its owner: impersonating one needs a SHA-256 second preimage.
- Signatures are hybrid and conjunctive: Ed25519 **and** ML-DSA-65 must both verify, over a message that includes the identifier.
- A contact card signs the session's transcript hash, so it cannot be replayed into another conversation.

## Files

| How a file travels | Encryption | Same as a message? |
|---|---|---|
| In a conversation, peer to peer | The session itself: X-Wing handshake, a key per frame, a post-quantum rekey every 5 minutes | Identical: same keys, same forward secrecy |
| In a conversation, through a sealed copy (no direct path) | ChaCha20-Poly1305 under a random 256-bit file key; the key only travels inside the session | The key has the session's protection; the stored blobs are only as long-lived as the conversation (deleted when it ends) |
| A drop link | The same AEAD under a random 256-bit file key, wrapped by a key derived from the link's 256-bit secret (and Argon2id of the passphrase) | Same ciphers and strength (256-bit keys, safe against quantum computers), but **no forward secrecy**: whoever later holds the link (and its passphrase) and a copy of the blobs can open them |
| A drop sealed to people | The file key wrapped by X-Wing to each recipient's prekey, signed with Ed25519 and ML-DSA-65 | Post-quantum like a contact handshake, and like its first message without forward secrecy: the prekeys derive from the identity seed |

Everything stored is ciphertext: file names, types, sizes and hashes are inside the encrypted manifest, every segment is a 4 MiB blob indistinguishable from random bytes (`drop::tests::nothing_in_clear`), and each drop has fresh keys. What a stored drop cannot have is forward secrecy, since nobody is online to agree on a fresh key: keep drop links short-lived and one-time, add a passphrase given another way, or send peer to peer only when that matters.

## Who sees what

| Observer | Sees | Never sees |
|---|---|---|
| GitHub Pages (the web server) | That someone loaded the app; IP address | The link secret (fragment), any message |
| Nostr relays / WebTorrent trackers | IP address, timing, a random-looking topic that changes daily, envelope sizes (padded) | Who talks to whom, names, content, IP of the other side |
| Blob servers (sealed drops) | IP addresses of the sender and of each recipient, when and how much (blobs of 4 MiB, the count of segments), how often a drop is fetched | Names, content, who it is for, the link or its passphrase (a drop sealed to identities does not name them) |
| STUN servers | IP address | Anything else |
| TURN server (if you set one) | IP addresses, sizes and timing of encrypted traffic | Content |
| The other person | Your IP address (unless you force TURN with "Always go through TURN"), your display name | Your vault, other conversations |
| Someone who obtains a link after it was used | A one-to-one link: nothing (it is consumed, and session keys never depended on the link alone). A group link: they can join until the room is locked or the link expires | Past messages |

## Verification

Compare the 5 emojis with the other person, by voice or in person:
- If they match, the session is end-to-end between you two.
- If they differ, someone is in the middle (it must hold the link). Press "They differ": the session ends.

Saved contacts were verified when they were saved, and reconnect with a mutually authenticated handshake (`KKpsk0 + X-Wing`) that needs both identities' keys and the contact secret.

## Limits (read these)

- **The code you run comes from GitHub Pages.** Whoever controls the GitHub account, the CI or a dependency could serve a modified app.
  - Mitigations: the service worker only updates when you accept, the site is built by public CI from this repository, and the CLI is an independent client of the same protocol.
  - For the highest stakes, use the CLI or a self-hosted copy.
- **Shared origin.** Every `maxgfr.github.io/*` project shares one web origin, so a page there (one of these sites, if it were compromised) is not kept out the way another site would be.
  - **Protected:**
    - the vault is always encrypted by your passphrase, which never leaves memory;
    - the offline copy of the app is checked file by file against hashes built into the service worker, so a rewritten cache is never served;
    - only hush's own pages can switch it to a new version.
  - **Not protected:**
    - such a page, if you visit it, can script a hush tab it opens;
    - it can read and change hush's plain-text preferences (relays, TURN server, display name). None of them protects a message, but a TURN server's login is among them: anyone holding it can use that relay. Give hush a login that is good for nothing else.
  - A dedicated domain would remove both.
- **Endpoints.** Malware, a malicious browser extension, or someone holding your unlocked phone sees what you see.
  - JavaScript strings and the browser's memory cannot be wiped reliably. "End" erases the session keys at once. Leaving the end screen reloads the page to drop the rest, except while the vault is unlocked (to keep contacts reachable): lock it, or close the tab.
- **A link intercepted before it is opened.**
  - If an attacker opens a one-to-one link first, they become the peer. The real recipient sees "link already used", and the emojis will not match if the attacker relays.
  - Send links through a channel you trust, keep the expiry short, add a passphrase given another way, and always compare the emojis.
- **The link lingers in the messenger you sent it through.** It stops working once used or expired.
- **Metadata.** Relays and trackers see IP addresses and timing.
  - Contacts' daily topics are unlinkable to outsiders, but a relay could notice one IP subscribing to several of them.
  - Use TURN-only mode, a VPN or Tor at the system level if that matters.
- **Passphrases** are only as strong as their entropy, slowed by Argon2id. Someone holding a protected link can try passphrases offline against a captured first message. Use long, random passphrases.
- **Both apps must be open** for messages. There is no server and no push, so a message cannot reach a closed app.
- **Sealed drops wait on public servers.** When a file cannot go peer to peer, its ciphertext waits on Blossom servers until it expires.
  - They see ciphertext only, under the same post-quantum keys, but they see IP addresses, sizes and timing, and they keep the bytes as long as they like: deletion is a request, and "one-time" is honored by clients, not enforced. Whoever later obtains the link (and its passphrase) and the bytes can open the drop.
  - A drop sealed to identities opens only with one of those identities' seeds; its link holds no key.
  - Send with "peer to peer only" (`--p2p-only`) or turn the servers off in Settings (`--no-servers`) to store nothing anywhere.
- **Group messages** are encrypted pairwise. A member who leaves keeps what they already received. Names are made unique within a group and renames are announced, but a name is still chosen by its owner: compare the emojis with each member.
- **Contacts.**
  - Contacts are saved only after both people compared the emojis. They reconnect with keys only they hold.
  - The first message of a reconnection carries no secret, but it has no forward secrecy: prekeys are derived from the identity seed.
  - People who connected by identifier are authenticated by that identifier, not by emojis.
- **Requests by identifier** carry no proof of who sent the envelope. Someone watching the relays who wins a race can make one request or lookup fail (never read or complete it), and a flood of lookups slows answers down.
- **Memory on phones and in the CLI.** The Rust engine does not wipe every secret from memory after use (decrypted vault records, contact secrets, passphrases). Lock the vault and quit the app when that matters.
- **The libraries.** The `ml-kem`, `ml-dsa` and `x-wing` crates are pure Rust (RustCrypto) and have not been independently audited. The versions used include every published fix. Constant-time behavior is not guaranteed under a JavaScript engine's JIT.
- **This code has been reviewed by its authors, not independently.** See [docs/AUDIT.md](docs/AUDIT.md) for what was checked, against which literature, and what was fixed.

## Phones

The Android and iOS apps run the same engine as the CLI, natively (no JavaScript). Over Bluetooth and Wi-Fi they carry the same sealed envelopes and frames as the relays: a nearby link adds reach, not trust. See [docs/MOBILE.md](docs/MOBILE.md) for what stays on the phone, and what a nearby link reveals:
- a rotating token while advertising;
- a `_hush._tcp` service on a shared Wi-Fi network;
- on Android over Wi-Fi Direct, the phone's Wi-Fi Direct device name.

## Browser hardening

- A strict Content-Security-Policy: no inline script, `connect-src 'self' wss:`.
- Trusted Types.
- No third-party resource.
- The app refuses to run inside a frame.
- Received files that are not previewable images are handed over as opaque downloads, never rendered.
- Test overrides in the address (`?relay=`) only work on `localhost`.
- No referrer is sent.
- The link fragment is removed from the address bar as soon as it is read.
