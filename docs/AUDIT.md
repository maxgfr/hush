# Security audit, October 2026

An internal review of hush: protocol, Rust core, web app and CLI. Each part was compared against the literature and specifications it builds on. Findings were fixed in the same change as this document. This is not an independent audit: the same people wrote and reviewed the code. The limits that remain are listed at the end and in [SECURITY.md](../SECURITY.md).

## Method

- **Read every module of `crates/hush-core`** against its reference:
  - Noise rev. 34 [1] and PQNoise [2] for the handshakes;
  - the X-Wing draft and paper [3], FIPS 203 and FIPS 204 [4] for the primitives;
  - ZRTP and Vaudenay's SAS model [5] for the emojis;
  - the Signal Double Ratchet specification [6] for the session;
  - Bindel and Hale [7] for the hybrid signatures;
  - "Taming the many EdDSAs" [8] for Ed25519 verification;
  - RFC 9106 [9] for Argon2;
  - PURBs/Padmé [10] for padding.
- **Reviewed the application layer:** `web/src`, `crates/hush-wasm` and `crates/hush-cli`. This covered how verification gates trust, untrusted strings, files, resource limits and same-origin exposure.
- **Dependencies:**
  - `cargo audit` and `cargo deny` are now a CI job (`deny.toml`);
  - known advisories for the post-quantum crates were checked by hand (below).
- **Fuzzing (`crates/hush-core/fuzz`):** every parser of untrusted input (envelopes, bundles, cards, links, identifiers, base64, padding, backups), byte-stream and BLE framing, handshake messages, and a stateful session target. The session target drives two sides through sends, losses, reordering, replays, corruption, rekeys and time jumps, and checks that what arrives intact decrypts to what was sent. Each target ran 10 minutes locally with no crash: 50 million inputs for the parsers, 16 million for the framing, 2.5 million for the handshakes, 360,000 for the session. CI runs each for a minute on every change.

## Findings

| # | Severity | Finding | Status |
|---|---|---|---|
| F1 | Critical | The emojis could be ground by someone in the middle | Fixed (link v2) |
| F2 | High | Every first handshake message of a link reused one AEAD key and nonce | Fixed (link v2) |
| F3 | High | Other sites on `maxgfr.github.io` could poison the offline cache | Fixed |
| H1 | High | A contact card from an unverified peer could be saved, then shown as verified | Fixed |
| F4 | Medium | A burst of envelopes could evict a nonce from the replay guard | Fixed |
| M1 | Medium | Group members could take another member's name, even retroactively | Fixed |
| M2 | Medium | Peers could inject fake lines into the CLI's output | Fixed |
| M3 | Medium | A group link holder could make members open connections without bound | Fixed |
| M4 | Medium | Names inside signed bundles were not cleaned (terminal escapes, bidi) | Fixed |
| M5 | Medium | Kept history stored disappearing and deleted messages | Fixed |
| F5 | Low | A malformed rekey could consume a frame's key | Fixed |
| F6 | Low | A bundle could claim an arbitrarily long lifetime | Fixed |
| L1 | Low | Unlocking a group was silent; the CLI unlocked on a malformed message | Fixed |
| L2 | Low | `?relay=` test overrides were honoured on the real site | Fixed |
| L3 | Low | Contact requests never expired; lookups had no rate limit | Fixed |
| L5 | Low | Received HTML or SVG kept their type and could render in the app's origin | Fixed |
| L6 | Low | Images auto-downloaded from unverified peers, without a limit | Fixed |
| L7 | Low | Duplicates were detected on a prefix, so a relay could get a genuine envelope dropped | Fixed |
| L8 | Low | CLI downloads could follow a planted symlink or use a Windows device name | Fixed |
| L9 | Low | Any page of the origin could make the service worker activate an update | Fixed |
| F7 | Low | The web app keyed the contact handshake with the raw contact secret instead of the key derived from it (`PROTOCOL.md` §6) | Fixed |
| L10 | Low | In the web app, a peer could reuse the id of one of your messages, then delete yours from kept history | Fixed |
| L11 | Low | In the web app, a peer could pile up messages and file offers without bound, and have one file streamed many times at once | Fixed |
| L12 | Low | In the web app, a relay or tracker could grow memory: duplicates were forgotten only by age, frames had no size limit | Fixed |
| L13 | Low | The web app's TURN login sits in `localStorage`, which SECURITY.md said held no secrets | Documented |
| L14 | Low | In the web app, deleted, expired or dropped files kept their object URL, and your own stayed downloadable after they disappeared | Fixed |

### F1: the emojis could be ground (critical)

**What was wrong.** The 5 emojis were derived from the transcript hash at the end of the handshake, so they also covered HS3 (`handshake.rs`, `LinkJoinerReady::write_hs3` and `LinkCreator::read_hs3`). The commitment to the joiner's nonce only works if nothing its sender chooses after seeing the other side's nonce feeds the SAS. That is the property ZRTP's hash commitment provides [5, §4.4.1.1] and Vaudenay's SAS-MCA model formalizes. HS3 broke it: it also carries `payload3`, the SDP answer, which is free text.

**The attack.** Mallory holds the link and sits between J and C.
1. Mallory completes the handshake with J. J's emojis are now fixed.
2. Playing the joiner towards C, Mallory waits for HS2, which reveals `N_C`.
3. Mallory then tries `payload3` values offline until C's emojis equal J's. That takes about 2^30 HKDF evaluations: seconds on a GPU.
4. Both people see matching emojis, and Mallory reads everything.

**Fix.** The SAS is now derived from the transcript hash taken right after HS2 (`sas_hash`), together with `N_C` and `N_J`:
- that hash binds both KEM messages and `N_C`;
- `N_J` was committed in HS1;
- whatever is sent after a reveal no longer counts.

Regression test: `sas_ignores_what_follows_the_reveal`.

### F2: one key and nonce for every HS1 (high)

**What was wrong.** After `MixKey(psk)`, the handshake key depended only on the link and its passphrase, and HS1 was encrypted with nonce 0. Every HS1 of a link was therefore sealed under the same ChaCha20-Poly1305 key and nonce: each group member joining, and each retry.

Noise forbids exactly this. In a `psk` handshake the `e` token must also run `MixKey(e)` [1, §9.2], and the validity rule exists for this reason [1, §9.3]. PQNoise keeps the rule [2].

**Impact.** Someone holding the link but not its passphrase could:
- recover the Poly1305 key from two HS1s and forge an HS1 that C accepts, which consumes a protected one-to-one link (denial of service);
- read the XOR of the joiners' names.

**Fix.** `MixKey(e_J)` after `MixHash(e_J)`. Regression test: `each_first_message_has_its_own_key`.

The contact handshake is not affected: its first encryption key already includes the shared secret of a fresh encapsulation.

### F1 and F2 change the wire format: link version 2

Links now read `#2.…`, and the protocol name is `hush/v2/link:…`. A version 1 link is refused with a clear message ("comes from an older version") rather than failing as a wrong passphrase.

Nothing else changes:
- vaults, backups and saved contacts keep working;
- the contact handshake is unchanged.

### F3: offline cache poisoning (high)

**What was wrong.** Cache Storage belongs to the origin, and every `maxgfr.github.io/*` site shares it. Any of them could rewrite `hush-<version>`, for example by replacing the app shell with its own script. The service worker served the cache first, so the change was persistent.

**Fix.** The build embeds each file's SHA-256 in `sw.js`. The worker's script lives in its registration, which other pages cannot rewrite. The worker:
- checks every file it installs and every response it serves from the cache;
- on any mismatch, drops the cache and serves from the network;
- serves nothing from the cache that is not on its list.

The end-to-end test `sw.spec.ts` poisons the cache and checks that the genuine app loads.

**What remains** (in SECURITY.md): a page of the same origin that you visit can still script a hush tab it opens. Only a dedicated origin removes that.

### H1: cards before verification (high)

**What was wrong.** The "Save contact" menu required both sides to have compared the emojis, but an incoming card's banner did not. Someone in the middle could send its own card named after the real peer before the emojis were compared. If it was saved, it was then trusted, and shown as verified, on every reconnection.

**Fix.** Cards are held until both sides have verified. `offerCard`, `store` and card handling all check `isVerified` (`contacts.ts`).

### The other findings

- **F4.** The replay guard now remembers each nonce until its envelope can no longer pass the freshness window, even the 10-minute window of manual codes. It refuses new envelopes when full instead of forgetting old ones (`envelope.rs`).
- **F5.** `Session::open` checks a REKEY or REKEY_ACK before committing the chain state, as its documentation promised. Test: `malformed_rekey_leaves_the_state_untouched`.
- **F6.** A bundle claiming more than 14 days of validity is refused.
- **M1.** In a group, names are made distinct from the other members' (`Bob (2)`). A rename is announced, and past messages keep the name they were sent under (web and CLI).
- **M2.** The CLI indents continuation lines of a message.
- **M3.** At most 4 handshakes are in flight at once, and each expires after 45 s (web and CLI, both roles).
- **M4.** The core cleans bundle names for display (`bundle::display_name`): no control characters (so no terminal escapes), no zero-width or bidi-override characters. A saved contact keeps the name you saved, whatever its later cards say. `hush backup inspect` prints cleaned names.
- **M5.** Kept history:
  - never stores disappearing messages;
  - forgets deleted ones;
  - reads the history setting as it is now, not as it was when the chat started.
- **L1.** Locking and unlocking a group are announced. A malformed message no longer unlocks the CLI's room.
- **L2.** `?relay=`, `?tracker=`, `?notracker` and `?nostun` only work on `localhost`.
- **L3.** Contact requests expire with the requester's wait (5 minutes). The app answers at most one lookup a second.
- **L5.** Only previewable raster images (png, jpeg, gif, webp, avif) keep their type. Everything else becomes `application/octet-stream`.
- **L6.** Images only download by themselves from a verified peer, three at a time at most.
- **L7.** Envelopes are deduplicated on a hash of the whole envelope, in the web app and the CLI.
- **L8.** The CLI creates downloads with `create_new` and never opens an existing path, which also rules out a planted symlink. Windows device names and trailing dots or spaces are neutralized.
- **L9.** The service worker only takes `skip-waiting` from pages in its own scope.
- **F7.** The same contact secret was used directly as the handshake's pre-shared key and as the input that derives the rendezvous keys, against the protocol's own key separation. It was found when the Rust engine (which follows the specification) could not reconnect with a browser. The web app now derives the key as specified; its end-to-end tests run against the Rust engine.
- **L10.** Message ids are chosen by their sender. The web app drops a message or file whose id is already shown, whoever sent it. Only a message's sender can delete it, and kept history then forgets only that author's entry (`removeHistory(key, id, mine)`). History never stores an id twice. Tests: `chat.test.ts`.
- **L11.** A conversation keeps its latest 2000 items (`MAX_ITEMS`), as kept history does. A file is streamed to a peer once at a time, however often it is asked for.
- **L12.** The duplicate filter remembers at most 4096 envelopes, oldest out first; the core's replay guard still refuses a copy. Relay and tracker frames over 512 KiB are dropped before they are parsed (`MAX_FRAME`): the largest envelope, one 256 KiB session frame, is about 372 KB.
- **L13.** The login is documented as readable by the origin's other pages (SECURITY.md, and the settings screen). It only gives access to that TURN server, which sees nothing but ciphertext. Keeping it in memory only would mean typing it at every visit.
- **L14.** Whenever a message leaves (deleted, expired, dropped by the limit, or the conversation ends), its file's object URL is revoked, a download under way is dropped, and your own file is no longer offered: a stream already started stops.

## Checked and sound

- **Primitives and versions.**
  - `ml-dsa 0.1.1` includes the fixes for:
    - the Decompose timing leak (RUSTSEC-2025-0144 / CVE-2026-22705, fixed in 0.1.0-rc.2) [11];
    - the repeated-hint malleability (GHSA-5x2r-hc65-25f9);
    - the `use_hint` off-by-two (GHSA-h37v-hp6w-2pp8, fixed in 0.1.0-rc.5).
  - No advisory affects `ml-kem 0.3.3` or `x-wing 0.1.1`. KyberSlash concerned the unmaintained `pqc_kyber`, which hush does not use.
  - `cargo audit` reports one informational item: `basic-toml` is unmaintained. It is used at build time by UniFFI, and the exception is recorded in `deny.toml`.
- **X-Wing** [3] is used as specified:
  - 32-byte seeds;
  - non-contributory X25519 results are rejected;
  - one ciphertext per encapsulation.

  Its security needs only one of X25519 and ML-KEM-768 to hold.
- **Hybrid signatures** are conjunctive (both must verify). The signed message starts with `hush/v1/sig` and the signer's identifier hash, which commits to both public keys. ML-DSA uses a context string. A stripped or transplanted component therefore fails, which is the non-separability Bindel and Hale ask for [7]. ML-DSA signs hedged (FIPS 204's default). Ed25519 verifies with `verify_strict`, which rejects small-order keys and non-canonical encodings [8].
- **Contact handshake** (`KKpsk0` with KEMs, as in PQNoise's KK [2]):
  - C1 can only be read with R's prekey;
  - C2 can only be read with I's ephemeral and prekey;
  - C3 proves I read C2.

  Stealing one side's keys does not let an attacker impersonate the other side to it: the KEM version of KK resists key compromise impersonation, unlike the DH one. Requests by identifier bind the requester's bundle to the identifier hash it claims, and trust it only after C3.
- **Session:**
  - one key per message;
  - counter nonces under unique keys;
  - the header and session id are authenticated;
  - skipped keys are bounded (1000 per chain, as the Double Ratchet recommends [6]);
  - replays, reflections and moves to another lane fail;
  - a post-quantum rekey every 5 minutes or 1000 messages gives post-compromise security against a passive attacker.
- **Padding:** Padmé [10] with a 256-byte floor; it leaks at most O(log log n) bits of length.
- **Passphrases:** Argon2id with 64 MiB and 3 passes, RFC 9106's second recommended setting with one lane [9]. Costs read from files are bounded.
- **Strings from peers** are rendered as text only (no `innerHTML`). Trusted Types and a strict CSP are in place. Downloads are checked against their SHA-256 before they can be opened.

## Limits that remain (documented in SECURITY.md)

- **Same origin.** A page of `maxgfr.github.io` that you visit can script a hush tab, and can read and change hush's plain-text settings in `localStorage`: relays, TURN server, display name. None of them protects a message, and a TURN server only ever sees encrypted traffic, but its login is there too: anyone holding it can use that relay. A dedicated origin would remove both.
- **Contacts' first message.** The payload of a contact handshake's C1 has no forward secrecy (as in Noise's KK). Prekeys are derived from the identity seed, so a stolen seed exposes every C1 sent to it. C1 only carries a name and, for requests, the requester's public bundle.
- **Saved contacts reconnect as verified** because they were verified when saved. People who connected by identifier are authenticated by the identifier, which the requester typed and the owner sees.
- **Bluetooth and Wi-Fi (mobile).**
  - Discovery tokens rotate every 10 minutes, but not in step with the phone's Bluetooth address. Someone nearby can therefore follow a device while it advertises (Becker et al. [12]). Apps only advertise while a conversation waits for someone.
  - On a local network, the `_hush._tcp` service shows that someone uses hush.

## References

1. T. Perrin, *The Noise Protocol Framework*, revision 34 (2018), §5.2, §9.2, §9.3.
2. Y. Angel, B. Dowling, A. Hülsing, P. Schwabe, F. Weber, *Post Quantum Noise*, ACM CCS 2022.
3. M. Barbosa, D. Connolly, J. Duarte, A. Kaiser, P. Schwabe, K. Varner, B. Westerbaan, *X-Wing: The Hybrid KEM You've Been Looking For*, IACR CiC 1(1), 2024; draft-connolly-cfrg-xwing-kem-10.
4. NIST FIPS 203 (ML-KEM) and FIPS 204 (ML-DSA), August 2024.
5. RFC 6189, *ZRTP*, §4.4.1.1; S. Vaudenay, *Secure Communications over Insecure Channels Based on Short Authenticated Strings*, CRYPTO 2005.
6. T. Perrin, M. Marlinspike, *The Double Ratchet Algorithm*, Signal (2016).
7. N. Bindel, B. Hale, *A Note on Hybrid Signature Schemes*, IACR ePrint 2023/423.
8. K. Chalkias, F. Garillot, V. Nikolaenko, *Taming the many EdDSAs*, SSR 2020.
9. RFC 9106, *Argon2*, §4.
10. K. Nikitin et al., *Reducing Metadata Leakage from Encrypted Files and Communication with PURBs*, PETS 2019.
11. RUSTSEC-2025-0144; GHSA-5x2r-hc65-25f9; GHSA-h37v-hp6w-2pp8.
12. J. K. Becker, D. Li, D. Starobinski, *Tracking Anonymized Bluetooth Devices*, PETS 2019.
