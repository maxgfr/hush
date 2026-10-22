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

## Who sees what

| Observer | Sees | Never sees |
|---|---|---|
| GitHub Pages (the web server) | That someone loaded the app; IP address | The link secret (fragment), any message |
| Nostr relays / WebTorrent trackers | IP address, timing, a random-looking topic that changes daily, envelope sizes (padded) | Who talks to whom, names, content, IP of the other side |
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
    - it can change hush's plain-text preferences (relays, TURN server, display name), which never hold secrets.
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
- **Both apps must be open.** There is no server and no push, so a message cannot reach a closed app.
- **Group messages** are encrypted pairwise. A member who leaves keeps what they already received. Names are made unique within a group and renames are announced, but a name is still chosen by its owner: compare the emojis with each member.
- **Contacts.**
  - Contacts are saved only after both people compared the emojis. They reconnect with keys only they hold.
  - The first message of a reconnection carries no secret, but it has no forward secrecy: prekeys are derived from the identity seed.
  - People who connected by identifier are authenticated by that identifier, not by emojis.
- **The libraries.** The `ml-kem`, `ml-dsa` and `x-wing` crates are pure Rust (RustCrypto) and have not been independently audited. The versions used include every published fix. Constant-time behavior is not guaranteed under a JavaScript engine's JIT.
- **This code has been reviewed by its authors, not independently.** See [docs/AUDIT.md](docs/AUDIT.md) for what was checked, against which literature, and what was fixed.

## Phones

The Android and iOS apps run the same engine as the CLI, natively (no JavaScript). Over Bluetooth and Wi-Fi they carry the same sealed envelopes and frames as the relays: a nearby link adds reach, not trust. See [docs/MOBILE.md](docs/MOBILE.md) for what stays on the phone, and what a nearby link reveals:
- a rotating token while advertising;
- a `_hush._tcp` service on a shared Wi-Fi network.

## Browser hardening

- A strict Content-Security-Policy: no inline script, `connect-src 'self' wss:`.
- Trusted Types.
- No third-party resource.
- The app refuses to run inside a frame.
- Received files that are not previewable images are handed over as opaque downloads, never rendered.
- Test overrides in the address (`?relay=`) only work on `localhost`.
- No referrer is sent.
- The link fragment is removed from the address bar as soon as it is read.
