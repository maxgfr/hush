# Mobile apps: Android and iOS, with Bluetooth and Wi-Fi

The phone apps are the web app's equals: they open the same links, take part in the same groups, save the same contacts, and read the same backups. They can also do what a browser cannot: find people nearby over Bluetooth LE or Wi-Fi, with no network at all.

## Architecture

```
SwiftUI / Jetpack Compose        screens, the Bluetooth and Wi-Fi radios
        │  UniFFI (crates/hush-ffi: HushApp, AppEvent, LocalTransport)
crates/hush-app                  the engine, shared with the CLI
        │                        conversations, contacts, vault, Nostr, trackers, WebRTC, nearby links
crates/hush-core                 the protocol (handshakes, ratchet, envelopes, identities)
```

- The apps hold no protocol logic. They show what the engine says (`AppEvent`) and move bytes for the radios (`LocalTransport`).
- The engine does the rest:
  - handshakes, sessions and framing;
  - choosing the path to each peer: a nearby link, then WebRTC, then the relays;
  - the vault on disk, with the same records and backups as the web app.
- The engine's tests (`crates/hush-app/tests`) run whole conversations over a simulated radio:
  - byte streams and 185-byte Bluetooth packets, with no network;
  - an in-process relay with WebRTC.

  The end-to-end suite runs the same engine against the browser (`web/e2e/app.spec.ts`).

## Transports

| Transport | Discovery | Data | Between |
|---|---|---|---|
| Internet | Nostr relays, WebTorrent trackers | WebRTC, or relayed | Everyone (phones, browsers, CLI) |
| Bluetooth LE | Advertised service UUID = discovery token | L2CAP channel; GATT packets as a fallback | iPhone ↔ Android, either way |
| Wi-Fi, same network | DNS-SD `_hush._tcp`, instance = token | TCP | iPhone ↔ Android |
| Wi-Fi, no router | Apple peer-to-peer Wi-Fi / Wi-Fi Direct | TCP | iPhone ↔ iPhone, Android ↔ Android |

`PROTOCOL.md` §8 specifies the details:
- tokens: `HMAC(env_key, "hush/v2/local/adv" ‖ slot)`, rotating every 10 minutes;
- who advertises and who scans;
- the GATT service and its characteristics;
- framing, and lane 255 for handshakes.

**When the apps advertise and scan:**
- A link you created is advertised until someone joins. A group link is advertised until it is locked or full.
- A link you opened is scanned for until you are connected.
- Saved contacts not connected: the side with the smaller identifier hash scans and starts the handshake; the other advertises.
- Nothing else is ever advertised.

## Security on the phones

Everything in `SECURITY.md` holds. On top of it:
- **Vault:** it lives in the app's private storage.
  - iOS: data protection `complete`, excluded from iCloud backups.
  - Android: `allowBackup="false"` and backup rules that exclude everything.
- **Backups:** an exported backup can be copied anywhere and attacked offline, so on Android its passphrase follows the vault's rules: at least 10 characters, typed twice.
- **Screens:** they are hidden from screenshots and the app switcher (Android `FLAG_SECURE`, iOS overlay when inactive).
- **Logs:** no link, message, name or key is ever logged.
- **Received files:** they stay in the app's private storage. They leave it only when you open or share one.
  - Android opens one, in an app you pick, with the type of its own extension, never the type the sender declared. Web pages, SVG, XML, scripts, packages and unknown types go out as opaque files.
- **Clipboard:** copied links are marked sensitive (Android) or local and expiring (iOS).
  - Android clears a copied link after a minute, if it is still what the clipboard holds.
  - The link and the identifier are not selectable: every copy goes through the Copy button.
- **Keyboard (Android):** text fields ask the keyboard not to learn from what is typed (`IME_FLAG_NO_PERSONALIZED_LEARNING`), and link fields turn autocorrect off. Passphrases, pasted links and drafts are never kept in the saved instance state.
- **Links from other apps:** a link opened from a browser or a messenger, or shared as text, only fills the Join screen. Nothing is joined until you tap Join.
  - Android does not verify the app link (there is no `assetlinks.json` to publish), so a link opens in the app only if you pick it, and otherwise in the browser, which runs the web app.
  - Either way the key is in the `#fragment`, which is never sent to the web server.
- **Task hijacking:** the Android activity has no task affinity, so another app cannot slip its own screens into hush's task.
- **Nearby links add reach, not trust:** they carry the same sealed envelopes and frames as the relays.
  - A link that keeps sending garbage is closed.
  - At most 8 are open at once.
  - Records are bounded.
- **What a nearby link reveals:**
  - Someone scanning around you sees a random-looking token while the app advertises. It changes every 10 minutes, but not in step with the phone's Bluetooth address, so they could follow your phone while it advertises (Becker et al., PETS 2019).
  - On a shared Wi-Fi network, `_hush._tcp` shows that someone uses hush.
  - While an Android phone advertises or scans over Wi-Fi Direct, it is discoverable under its Wi-Fi Direct device name (often the phone's model or its owner's name), next to `<token>._hush._tcp`. A neutral name in the Wi-Fi Direct settings avoids that.
  - The token is sent in the clear, so anyone in range can advertise it too. They cannot finish the handshake without the link, and the app dials each device found on its own, so a copy cannot block the real one.
  - Joining a Wi-Fi Direct group puts the phone on the network of the group's owner. Android only does it when Bluetooth and the local network found nothing for 15 s, one group at a time, and leaves the group as soon as its link closes.

## Building

The Rust engine is cross-compiled and its bindings generated by:
- `mobile/scripts/build-android.sh`, which needs the NDK and `cargo-ndk`;
- `mobile/scripts/build-ios.sh`, which needs Xcode.

The CI workflow runs both, alongside the web, Rust and CLI jobs, and builds and tests the apps:
- **Android:** unit tests, lint, and instrumented tests on an emulator.
- **iOS:** `swift test` for HushKit on macOS, and unit and UI tests on a simulator.

Every run, on a pull request as on `main`, publishes the apps as artifacts at the bottom of its page (Actions → the run → Artifacts), kept 30 days:
- `hush-android-debug`: `hush-android-debug.apk`, the debug build. It is called "hush debug" and has its own id (`io.github.maxgfr.hush.debug`), so it installs next to the release app.
- `hush-ios`:
  - `hush-ios-simulator.zip`, for a simulator on an Apple silicon Mac (`xcrun simctl install booted Hush.app`);
  - `hush-ios-unsigned.ipa`, the iPhone build, unsigned: a sideloading tool (Sideloadly, AltStore) signs it with your Apple account before installing.

`gh run download <run id> -n hush-android-debug` fetches one from the terminal.

A release only goes out once every job is green, and it carries the signed Android app (`hush-android.apk`) next to the CLI binaries.

**Install a release**
- **Android:** download `hush-android.apk` from the [latest release](https://github.com/maxgfr/hush/releases/latest) and open it on the phone (allow installing from your browser). Check it against `SHA256SUMS`. Later releases install over it, since they are signed with the same key, which CI holds as the `ANDROID_KEYSTORE` secrets.
- **iPhone:**
  1. Run `mobile/scripts/build-ios.sh`.
  2. Run `xcodegen generate` in `mobile/ios`.
  3. Open `Hush.xcodeproj`, choose your Apple account under Signing, and run on your phone.

  A free account installs for 7 days; TestFlight or the App Store need a paid one.

## Testing on two real phones

Emulators and simulators have no Bluetooth and no peer-to-peer Wi-Fi, so this part can only be checked by hand. With two phones (ideally one iPhone and one Android), and the app open on both:

1. **Bluetooth, no network.** Both phones in airplane mode with Bluetooth on.
   1. A creates a link and shows its QR code; B scans it.
   2. Both show the same 5 emojis and a "Bluetooth" badge.
   3. Compare the emojis, then exchange messages and a photo of a few MB.
   4. End: B shows "the other person ended the conversation".
2. **Wi-Fi.** Both phones on the same Wi-Fi network, Bluetooth off. Same steps, with a "Wi-Fi" badge.
3. **Wi-Fi without a router.** Two iPhones (or two Android phones), no shared network, Wi-Fi on. Same steps.
4. **Groups.** Create a group and have three phones join, mixing Bluetooth and the Internet. Check that everyone sees everyone, then lock the group.
5. **Contacts nearby.**
   1. Create a vault on both, compare the emojis, and save each other.
   2. End the conversation. Go into airplane mode with Bluetooth on.
   3. Open Contacts: the contact comes online and the chat connects over Bluetooth.
6. **Back to the Internet.** In a Bluetooth conversation, walk out of range with the Internet on. After about 20 s the badge turns to "Relayed" or "Direct" and messages still arrive.
7. **Interop with the web.** Open a phone's link in a browser, and a browser's link on a phone, over the Internet. Then add each other by identifier.
8. **Hostile cases.**
   1. Open a one-to-one link on a third phone first: the intended recipient sees "link already used".
   2. Answer "They differ": the conversation ends at once.

Report anything that fails with the platform, OS version and step.

## Not yet

- **Mailboxes:** messages to someone who is not there right now, and Bridgefy-style store-and-forward between phones. Today's ratchet assumes both sides are online (see SPQR for an asynchronous one).
- **Background delivery:** the apps keep links alive while open. iOS limits Bluetooth in the background.
