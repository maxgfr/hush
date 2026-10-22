/**
 * Preferences (never secrets): which relays, trackers and STUN/TURN servers
 * to use, a display name, the default message timer. Kept in localStorage.
 */
import { signal } from "@preact/signals";
import { DEFAULT_STUN } from "../net/peer";
import { DEFAULT_RELAYS, DEFAULT_TRACKERS } from "../signal/defaults";
import type { SignalConfig } from "../signal/mux";

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

export function saveSettings(next: Partial<Settings>): void {
  settings.value = { ...settings.value, ...next };
  try {
    localStorage.setItem(KEY, JSON.stringify(settings.value));
  } catch {
    // Private mode: preferences last for this tab only.
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
  return out;
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
