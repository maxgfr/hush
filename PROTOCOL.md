# hush protocol

Links are at version 2. Version 2 changed the link handshake (§3) after the [October 2026 audit](docs/AUDIT.md), and version 1 links are refused. Every other format is unchanged since version 1.

Everything here is implemented once, in `crates/hush-core`. The web app (through WebAssembly), the CLI and the mobile bindings call the same code. This document is the reference for anyone writing another client.

There is a single cipher suite and nothing is negotiated, so nothing can be downgraded.

- `H` = SHA-256.
- `Extract(salt, ikm)` and `Expand(prk, info, L)` are HKDF-SHA-256 (RFC 5869).
- `AEAD` = ChaCha20-Poly1305 (RFC 8439). `XAEAD` = XChaCha20-Poly1305.
- `KEM` = X-Wing (X25519 + ML-KEM-768, draft-connolly-cfrg-xwing-kem). Encapsulation key 1216 bytes, ciphertext 1120 bytes, shared secret 32 bytes. Non-contributory X25519 results are rejected.
- Every label is an ASCII string starting with `hush/v1/`, except the link handshake's protocol name (`hush/v2/…`).
- Integers are big-endian.
- `b64u` is unpadded base64url with canonical trailing bits.

## 1. Links

```
https://maxgfr.github.io/hush/#2.<mode>.<b64u(L)>.<expiry>[.p]
mode    s (one-to-one) | g (group)
L       32 random bytes
expiry  minutes since the Unix epoch, base 36, no leading zero
.p      the link needs a passphrase
```

```
root    = Extract("hush/v1/link", L)
topic_d = Expand(root, "hush/v1/link/topic" || u32(d), 32)      d = floor(unix_ms / 86400000)
env_key = Expand(root, "hush/v1/link/env", 32)
room_id = Expand(root, "hush/v1/link/room", 16)
psk     = Expand(root, "hush/v1/link/psk", 32)
        or, with a passphrase P (NFKC-normalized):
psk     = Expand(Extract(root, Argon2id(P, salt = H("hush/v1/link/pass" || L)[..16], m = 64 MiB, t = 3, p = 1)), "hush/v1/link/psk", 32)
```

Clients listen on the topics of today and yesterday.

## 2. Envelopes (signaling)

```
envelope = 0x01 || nonce(24) || XAEAD(env_key, nonce, aad = "hush/v1/env" || 0x01 || topic, pad(inner))
inner    = ts_ms(u64) || from(16) || to(16, all zero = everyone) || kind(1) || body
```

- **Validity:** an envelope is accepted once and only within ±120 s of `ts_ms`, or ±600 s when it was carried by hand as a code. Its nonce is remembered until `ts_ms + 600 s`. When too many fresh nonces are pending, new envelopes are refused rather than old nonces forgotten.
- **Tags:** `from` and `to` are random tags, one per app session. They never identify a person.
- **Kinds:**

| kind | name | body |
|---|---|---|
| 1, 2, 3 | HS1, HS2, HS3 | link handshake messages (§3) |
| 4 | SESSION | a session frame relayed when no direct path exists (§4) |
| 5 | TAKEN | empty: "this one-to-one link is already used" |
| 6 | HERE | contact presence: the sender's identifier hash |
| 7, 8, 9 | C1, C2, C3 | contact handshake messages (§6) |
| 10 | LOOKUP | empty: "who owns this identifier?" |
| 11 | BUNDLE | the owner's signed bundle (§5) |
| 12 | REQUEST | `requester_id_hash(32) ‖ pow_nonce(u64) ‖ C1` |

**Transports.** These carry `b64u(envelope)` and never look inside:
- **Nostr:** ephemeral events of kind 21918, tagged `["t", hex(topic)]` and signed with a throwaway secp256k1 key.
- **WebTorrent trackers:** the `info_hash` is `topic[..20]`, and the envelope goes in an offer's `sdp` field.
- **By hand:** `hushcode:` + b64u(envelope), as a QR code or text.

**Padding.** `pad(x) = u32(len(x)) || x || zeros`, up to `max(256, Padmé(len + 4))`.

## 3. Link handshake: `NNpsk0 + X-Wing`

This is a Noise-style symmetric state (§5.2 of the Noise spec): `h`, `ck`, `k`, `n`.
- **MixKey:** `MixKey(ikm)` sets `ck, k = Expand(Extract(ck, ikm), "hush/v1/noise/mix", 64)`.
- **Encrypt:** `EncryptAndHash` uses `AEAD(k, nonce = 0^4 || u64(n), aad = h)`.
- **Protocol name:** `hush/v2/link:NNpsk0+XWing+SHA256+ChaChaPoly`.

J opens the link; C is a member (the creator, or in a group any member).

```
h = ck = H(protocol name)
MixHash("hush/v1/link/prologue" || mode || room_id || u64(expiry)); MixKey(psk)

HS1  J → C   e (X-Wing ek)           MixHash(e); MixKey(e)
             EncryptAndHash(H("hush/v1/sas-commit" || N_J) || payload1)
HS2  C → J   ct = Encap(e)           MixHash(ct); MixKey(ss)
             EncryptAndHash(N_C || payload2)        h2 = h after this message
HS3  J → C   EncryptAndHash(N_J || payload3)        C checks the commitment to N_J
root = Expand(ck, "hush/v1/noise/split" || h, 32)
SAS  = 5 × 6 bits of Expand(Extract("hush/v1/sas", h2), "hush/v1/sas/emoji" || N_C || N_J, 4)   (Matrix's 64 emojis)
```

Two rules make this safe:
- **`MixKey(e)`.** This is Noise's rule for `psk` handshakes: it gives every HS1 its own key. Without it, every HS1 of a link would be encrypted under one key and nonce.
- **The SAS covers the transcript up to HS2 only.** Each side's nonce is fixed before the other side's is known, and nothing sent after a reveal changes the emojis. Someone in the middle therefore gets one try, which succeeds with probability 2^-30.

The payloads are UTF-8 JSON:
- HS1: `{"v":1,"name":…}`
- HS2: `{"v":1,"name":…,"sdp":<offer>}`
- HS3: `{"v":1,"sdp":<answer>}`

**SDP.** The SDP is non-trickle: ICE gathering is complete before it is sent.

**Who answers.**
- **One-to-one link:** C answers the first HS1 only. Later HS1s get TAKEN.
- **Group link:** every member answers a new HS1, which forms a full mesh. Once locked, members stop answering.

## 4. Session

```
epoch0   = Expand(root, "hush/v1/session/epoch0", 32)
id       = Expand(root, "hush/v1/session/id", 16)
contact  = Expand(root, "hush/v1/session/contact", 32)    contact secret, if cards are exchanged
signal   = Expand(root, "hush/v1/session/signal", 32)     env key and topics for relayed frames
chain    = Expand(epoch_root, "hush/v1/session/chain" || dir || lane, 32)
           dir: 0 = initiator → responder (J → C), 1 = the other way
mk_n     = HMAC(chain, 0x01);  chain = HMAC(chain, 0x02)
frame    = 0x01 || lane(1) || epoch(u32) || ctr(u64) || AEAD(mk_ctr, 0^4 || u64(ctr), aad = header || id, pad(type(1) || body))
```

**Lanes.**
- 0: control.
- 1: chat.
- 2: file.
- 3: relayed signaling.

On WebRTC, lanes 0, 1 and 2 are the negotiated DataChannels with ids 0, 1 and 2 (`ctrl`, `chat`, `file`).

**Types.**
- `0` application data.
- `1` REKEY: `u32(next) || X-Wing ek`.
- `2` REKEY_ACK: `u32(next) || ct`.
- `3` BYE.

**Rekey.**
1. The initiator sends REKEY every 5 minutes or 1000 messages.
2. The responder encapsulates and answers REKEY_ACK, still in the old epoch.
3. Both then use `epoch_root' = Extract(epoch_root, ss || "hush/v1/session/rekey" || u32(next))`.
4. The previous epoch's receive chains are kept for 120 s, then erased.

**Rejected frames.** These leave the state untouched:
- a replayed counter;
- a counter more than 1000 ahead;
- a frame from an unknown epoch;
- any AEAD failure.

**Application messages** (UTF-8 JSON, lanes 0 and 1):

```
ctrl: {"t":"name","name"} {"t":"verified"} {"t":"lock","locked"} {"t":"card","card":b64u}
chat: {"t":"msg","id","ts","text","ttl"?} {"t":"del","id"}
      {"t":"file","id","ts","name","size","mime","sha256","ttl"?} {"t":"file-get","id"} {"t":"file-end","id"} {"t":"file-cancel","id"}
file: 0x01 || id(16 ASCII) || u32(index) || ≤ 48 KiB of data          (lane 2, in order)
```

- `id` is 16 base64url characters.
- `ttl` is in seconds (at most 30 days) and both sides delete the message when it runs out.
- The receiver checks the SHA-256, then sends `file-end`.

## 5. Identities, bundles, cards

```
seed (32) → ed25519 = Expand(seed, "hush/v1/identity/ed25519"), mldsa65 seed = Expand(seed, "hush/v1/identity/mldsa65")
spk(epoch) = X-Wing key from Expand(seed, "hush/v1/identity/xwing-spk" || u32(epoch))     epoch = day / 7
id_hash    = H("hush/v1/id" || ed_pk(32) || mldsa_pk(1952))
identifier = bech32m("hush", 0x00 || id_hash)                                               64 characters
sig(purpose, m) = Ed25519(M) || ML-DSA-65(M, ctx = "hush/v1")     M = "hush/v1/sig" || id_hash || u8(len) || purpose || m
bundle = 0x01 || ed_pk || mldsa_pk || u32(spk_epoch) || spk || u64(created) || u64(expires) || u8(n) || name || sig("bundle", …)
card   = u16(len) || bundle || sig("card", session_hash || H(bundle))
```

A card is only valid for the session whose transcript hash it signs.

## 6. Contacts and requests

```
croot   = Extract("hush/v1/contact", contact_secret)
topic_d = Expand(croot, "hush/v1/contact/topic" || u32(d));  env = Expand(croot, "hush/v1/contact/env");  psk = Expand(croot, "hush/v1/contact/psk")
```

**Reconnecting.** Online clients post HERE on each contact's topic. The side with the smaller `id_hash` starts the contact handshake, `KKpsk0 + X-Wing`, with protocol name `hush/v1/contact:KKpsk0+XWing+SHA256+ChaChaPoly`:

```
MixHash("hush/v1/contact/prologue" || min(id_a, id_b) || max(id_a, id_b)); MixKey(psk)
C1  I → R  u32(epoch_R) || e_I || ct1 = Encap(spk_R)     MixHash each; MixKey(ss1); EncryptAndHash(payload1)
C2  R → I  u32(epoch_I) || ct2 = Encap(e_I) || ct3 = Encap(spk_I)     MixKey(ss2); MixKey(ss3); EncryptAndHash(payload2 = {sdp offer})
C3  I → R  EncryptAndHash(payload3 = {sdp answer})
```

**Requests by identifier.** These use the same handshake with the lookup keys:

```
lroot = Extract("hush/v1/lookup", id_hash); topic_d, env and psk as above with "hush/v1/lookup/…"
pow   = smallest u64 n such that H("hush/v1/pow" || id_hash || u32(day) || H(C1) || u64(n)) has ≥ 20 leading zero bits
```

C1's payload carries the requester's bundle. The owner only answers C2 after the user accepts.

## 7. Vault and backups

```
vault header = "HUSHVT" || 0x01 || u32(m_kib) || u32(t) || u32(p) || salt(16) || nonce(24) || XAEAD(Argon2id(P), aad = header prefix, DEK)
record       = nonce(24) || XAEAD(Expand(DEK, "hush/v1/vault/records"), aad = "hush/v1/vault/rec" || kind || 0 || id, pad(json))
storage key  = hex(HMAC(Expand(DEK, "hush/v1/vault/index"), kind || 0 || id)[..16])
backup       = "HUSHBK" || 0x01 || u32(m_kib) || u32(t) || u32(p) || salt(16) || nonce(24) || XAEAD(Argon2id(P), aad = header, pad(snapshot))
snapshot     = {"v":1, "identity": {"seed": b64u, "name"} | null, "contacts": [{"id","key","name","bundle","secret","added","history"}], "history": {key: [{"id","mine","ts","text","expiresAt"?}]}}
```

Argon2id costs are read from the header. Costs above 1 GiB, 16 passes or 8 lanes are refused.

## 8. Nearby links (mobile)

The phone apps also find each other over Bluetooth LE and Wi-Fi, with no network at all. A nearby link carries exactly what the relays and WebRTC carry: it adds reach, never trust.

**Discovery tokens.** Every channel (a link, a saved contact) has a 16-byte token for each 10-minute slot:

```
token(slot) = HMAC(env_key, "hush/v2/local/adv" || u64(slot))[..16]      slot = floor(unix_ms / 600000)
```

Tokens derive from the envelope key, not the topic, so a relay cannot recognize a token it hears nearby. Scanners also accept the previous slot's token.

**Who advertises, who scans.** This avoids two devices dialing each other at once:

| Channel | Advertises | Scans |
|---|---|---|
| A link | Its creator (and the members of an open group) | Whoever opened it, until connected |
| A saved contact, not connected | The side with the larger identifier hash | The side with the smaller one, which also starts the contact handshake |

**Bluetooth LE.**
- The advertiser adds a GATT service whose UUID is the token, as a big-endian 128-bit UUID, and advertises that UUID.
- The service has three characteristics:

| Characteristic | UUID | Use |
|---|---|---|
| PSM | `e261fba8-836b-459c-84cc-e15cd0d124b4` | Read: the L2CAP PSM to connect to, `u16` big-endian |
| RX | `cb5f7ab3-a841-4a1b-9f80-5855197fd921` | Write without response: packets from the scanner |
| TX | `66c5f983-f8d0-406d-afb7-3cd1b35e1712` | Notify: packets to the scanner |

- The scanner connects and reads PSM. It then opens an L2CAP connection-oriented channel, which is a byte stream.
- When L2CAP is unavailable, the scanner falls back to RX/TX packets of at most `ATT MTU − 3` bytes.

**Wi-Fi.** A DNS-SD service `_hush._tcp`, whose instance name is the lowercase hex of the token, offers a TCP byte stream. It works:
- on a local network (Bonjour, Android NSD);
- between iPhones with no network (Apple peer-to-peer Wi-Fi);
- between Android phones with no network (Wi-Fi Direct service discovery, the group owner listening on the `port` given in a TXT record).

**Framing.**

| Link | Wire format |
|---|---|
| Byte streams (TCP over Wi-Fi, BLE L2CAP) | Each record is `lane(1) ‖ u32(len) ‖ data`. |
| Small packets (BLE GATT) | A record is split into packets of `flags(1: first = 1, last = 2) ‖ u16(seq) ‖ bytes`. A lost or reordered packet drops the partial record. |

**Lanes.**
- Lane 255 carries signaling: `topic(32) ‖ envelope`, the envelopes of §2, exactly as a relay would carry them. Handshakes run on it, and a peer whose handshake came over the link (or was heard on it) gets its session frames on the same link.
- Lanes 0 to 3 carry that peer's session frames (§4). When a handshake runs over a nearby link, the hello payloads carry no `sdp`: no WebRTC is set up.

**Limits.** Each device keeps at most 8 nearby links. A link that sends more than 32 records that fail to open is closed.
