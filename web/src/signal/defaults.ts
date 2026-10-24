/** Default public rendezvous endpoints (editable in Settings). Kept dependency-free for the initial bundle. */
export const DEFAULT_RELAYS = [
  "wss://relay.damus.io",
  "wss://nos.lol",
  "wss://relay.primal.net",
  "wss://nostr.mom",
];

/** Blossom servers that take anonymous uploads of opaque blobs, with CORS (checked October 2026). */
export const DEFAULT_BLOSSOM = ["nostr.download", "blossom.yakihonne.com", "files.sovbit.host"];

export const DEFAULT_TRACKERS = [
  "wss://tracker.openwebtorrent.com",
  "wss://tracker.webtorrent.dev",
];
