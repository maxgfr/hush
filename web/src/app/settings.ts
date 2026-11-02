/**
 * Preferences: which relays, trackers and STUN/TURN servers to use, a display
 * name, the default message timer. Kept in plain localStorage, which every page
 * of the origin can read: nothing here protects a message. The one credential,
 * a TURN login's password, is kept out of it, sealed in IndexedDB (secrets.ts);
 * it only lets someone use that relay (the UI says so).
 */
import { signal } from "@preact/signals";
import { DEFAULT_STUN } from "../net/peer";
import { DEFAULT_BLOSSOM, DEFAULT_RELAYS, DEFAULT_TRACKERS } from "../signal/defaults";
import type { SignalConfig } from "../signal/mux";
import { readSecret, writeSecret } from "./secrets";

export interface Settings {
  name: string;
  relays: string[];
  trackers: string[];
  nostr: boolean;
  torrent: boolean;
  stun: string[];
  turn: { url: string; username: string; credential: string } | null;
  /** Force TURN so the peer never learns our IP. */
  relayOnly: boolean;
  /** Default disappearing timer for new conversations, in seconds (0 = off). */
  ttl: number;
  /** Blossom servers keeping sealed copies of files (drops, and files without a direct path). */
  blossom: string[];
  /** Off: files only ever travel peer to peer. */
  blossomOn: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  name: "",
  relays: DEFAULT_RELAYS,
  trackers: DEFAULT_TRACKERS,
  nostr: true,
  torrent: true,
  stun: DEFAULT_STUN,
  turn: null,
  relayOnly: false,
  ttl: 0,
  blossom: DEFAULT_BLOSSOM,
  blossomOn: true,
};

const KEY = "hush.settings.v1";

function load(): Settings {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULT_SETTINGS;
    const v = JSON.parse(raw) as Partial<Settings>;
    return { ...DEFAULT_SETTINGS, ...v };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export const settings = signal<Settings>(
  typeof localStorage === "undefined" ? DEFAULT_SETTINGS : load(),
);

/** What localStorage gets: everything but the TURN credential. */
function persist(): boolean {
  const s = settings.value;
  const kept = s.turn ? { ...s, turn: { ...s.turn, credential: "" } } : s;
  try {
    localStorage.setItem(KEY, JSON.stringify(kept));
    return true;
  } catch {
    // Private mode or storage full: preferences last for this tab only.
    return false;
  }
}

/** False when the change could not be stored (it lasts until the page is reloaded). */
export function saveSettings(next: Partial<Settings>): boolean {
  settings.value = { ...settings.value, ...next };
  return persist();
}

const TURN_SECRET = "turn-credential";

/** Saves the TURN server, its credential sealed apart. False if any of it could not be kept. */
export async function saveTurn(turn: Settings["turn"]): Promise<boolean> {
  const stored = saveSettings({ turn });
  return (await writeSecret(TURN_SECRET, turn?.credential ?? "")) && stored;
}

/**
 * Restores the TURN credential at boot. One saved in clear by an earlier version is
 * moved to the sealed store, then dropped from localStorage.
 */
export async function loadTurnCredential(): Promise<void> {
  const turn = settings.value.turn;
  if (!turn) return;
  if (turn.credential) {
    if (await writeSecret(TURN_SECRET, turn.credential)) persist();
    return;
  }
  const credential = await readSecret(TURN_SECRET);
  if (credential && settings.value.turn === turn) {
    settings.value = { ...settings.value, turn: { ...turn, credential } };
  }
}

/**
 * Test and dev overrides: `?relay=ws://…&tracker=…&nostun` (used by the e2e suite). Only on
 * localhost: on the real site, a crafted link could otherwise send all of someone's signaling
 * to one relay of the sender's choosing.
 */
function overrides(s: Settings): Settings {
  if (typeof location === "undefined" || location.hostname !== "localhost") return s;
  const q = new URLSearchParams(location.search);
  const out = { ...s };
  const relays = q.getAll("relay");
  const trackers = q.getAll("tracker");
  if (relays.length) out.relays = relays;
  if (trackers.length) out.trackers = trackers;
  if (q.has("notracker")) out.torrent = false;
  if (q.has("nostun")) out.stun = [];
  const blossom = q.getAll("blossom");
  if (blossom.length) out.blossom = blossom;
  return out;
}

/** The blob servers to use (empty: none, files only travel peer to peer). */
export function blossomServers(): string[] {
  const s = overrides(settings.value);
  return s.blossomOn ? s.blossom : [];
}

export function signalConfig(): SignalConfig {
  const s = overrides(settings.value);
  return {
    nostr: { enabled: s.nostr, relays: s.relays },
    torrent: { enabled: s.torrent, trackers: s.trackers },
    manual: { enabled: true },
  };
}

export function iceServers(): RTCIceServer[] {
  const s = overrides(settings.value);
  const out: RTCIceServer[] = s.stun.length ? [{ urls: s.stun }] : [];
  if (s.turn?.url) {
    out.push({ urls: s.turn.url, username: s.turn.username, credential: s.turn.credential });
  }
  return out;
}
