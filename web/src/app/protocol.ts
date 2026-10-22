/**
 * Application messages, inside session frames. Control and chat lanes carry
 * UTF-8 JSON; the file lane carries binary chunks. The CLI speaks the same
 * format (crates/hush-cli/src/app.rs); see PROTOCOL.md.
 *
 * Everything decoded here comes from the peer: validate strictly, never trust.
 */
import { fromUtf8, utf8 } from "../lib/bytes";

/** Handshake payloads (encrypted inside HS1/HS2/HS3 and C1/C2/C3). */
export interface HelloPayload {
  v: 1;
  name?: string;
  sdp?: string;
  /** Contact requests carry the requester's bundle (base64url). */
  bundle?: string;
  note?: string;
}

export type CtrlMessage =
  | { t: "verified" }
  | { t: "name"; name: string }
  | { t: "card"; card: string }
  | { t: "lock"; locked: boolean };

export type ChatMessage =
  | { t: "msg"; id: string; ts: number; text: string; ttl?: number }
  | { t: "del"; id: string }
  | {
      t: "file";
      id: string;
      ts: number;
      name: string;
      size: number;
      mime: string;
      sha256: string;
      ttl?: number;
    }
  | { t: "file-get"; id: string }
  | { t: "file-end"; id: string }
  | { t: "file-cancel"; id: string };

export const MAX_TEXT = 16_000;
export const MAX_NAME = 64;
export const MAX_FILE = 4 * 1024 * 1024 * 1024;

const isStr = (v: unknown, max: number): v is string => typeof v === "string" && v.length <= max;
const isId = (v: unknown): v is string => typeof v === "string" && /^[A-Za-z0-9_-]{8,32}$/.test(v);
const isNum = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v) && v >= 0;
const isTtl = (v: unknown) => v === undefined || (isNum(v) && v <= 30 * 86_400);

export const encodeJson = (v: unknown): Uint8Array => utf8(JSON.stringify(v));

function parse(data: Uint8Array): Record<string, unknown> | null {
  try {
    const v: unknown = JSON.parse(fromUtf8(data));
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** Zero-width and bidirectional-override characters a peer could use to spoof the UI. */
const UNSAFE = new Set([
  0x200b, 0x200c, 0x200d, 0x200e, 0x200f, 0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x2066, 0x2067,
  0x2068, 0x2069, 0xfeff,
]);

/** Drops control, zero-width and bidi-override characters from a peer-chosen name. */
export function cleanName(s: string): string {
  return Array.from(s)
    .filter((ch) => {
      const c = ch.codePointAt(0) ?? 0;
      return c >= 0x20 && !(c >= 0x7f && c <= 0x9f) && !UNSAFE.has(c);
    })
    .join("")
    .trim()
    .slice(0, MAX_NAME);
}

export function decodeHello(data: Uint8Array): HelloPayload | null {
  const o = parse(data);
  if (o?.v !== 1) return null;
  const out: HelloPayload = { v: 1 };
  if (isStr(o.name, 256)) out.name = cleanName(o.name);
  if (isStr(o.sdp, 64_000)) out.sdp = o.sdp;
  if (isStr(o.bundle, 16_000)) out.bundle = o.bundle;
  if (isStr(o.note, 280)) out.note = o.note;
  return out;
}

export function decodeCtrl(data: Uint8Array): CtrlMessage | null {
  const o = parse(data);
  if (!o) return null;
  switch (o.t) {
    case "verified":
      return { t: "verified" };
    case "name":
      return isStr(o.name, 256) ? { t: "name", name: cleanName(o.name) } : null;
    case "card":
      return isStr(o.card, 16_000) ? { t: "card", card: o.card } : null;
    case "lock":
      return typeof o.locked === "boolean" ? { t: "lock", locked: o.locked } : null;
    default:
      return null;
  }
}

export function decodeChat(data: Uint8Array): ChatMessage | null {
  const o = parse(data);
  if (!o) return null;
  switch (o.t) {
    case "msg":
      if (!isId(o.id) || !isNum(o.ts) || !isStr(o.text, MAX_TEXT) || !isTtl(o.ttl)) return null;
      return o.ttl === undefined
        ? { t: "msg", id: o.id, ts: o.ts, text: o.text }
        : { t: "msg", id: o.id, ts: o.ts, text: o.text, ttl: o.ttl as number };
    case "del":
      return isId(o.id) ? { t: "del", id: o.id } : null;
    case "file": {
      if (
        !isId(o.id) ||
        !isNum(o.ts) ||
        !isStr(o.name, 512) ||
        !isNum(o.size) ||
        o.size > MAX_FILE ||
        !isStr(o.mime, 128) ||
        !(typeof o.sha256 === "string" && /^[0-9a-f]{64}$/.test(o.sha256)) ||
        !isTtl(o.ttl)
      )
        return null;
      const f: ChatMessage = {
        t: "file",
        id: o.id,
        ts: o.ts,
        name: cleanName(o.name.replace(/[/\\]/g, "_")) || "file",
        size: Math.floor(o.size),
        mime: /^[\w.+-]+\/[\w.+-]+$/.test(o.mime) ? o.mime : "application/octet-stream",
        sha256: o.sha256,
      };
      if (o.ttl !== undefined) f.ttl = o.ttl as number;
      return f;
    }
    case "file-get":
    case "file-end":
    case "file-cancel":
      return isId(o.id) ? { t: o.t, id: o.id } : null;
    default:
      return null;
  }
}

/** File chunks: `0x01 || id (16 ASCII bytes, a `newId()`) || index (u32 BE) || data`. */
export const CHUNK_SIZE = 48 * 1024;
const CHUNK_TAG = 1;

export function encodeChunk(id: string, index: number, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(21 + data.length);
  out[0] = CHUNK_TAG;
  if (id.length !== 16) throw new Error("file ids are 16 characters");
  out.set(utf8(id), 1);
  new DataView(out.buffer).setUint32(17, index);
  out.set(data, 21);
  return out;
}

export function decodeChunk(
  data: Uint8Array,
): { id: string; index: number; data: Uint8Array } | null {
  if (data.length < 21 || data[0] !== CHUNK_TAG) return null;
  let id: string;
  try {
    id = fromUtf8(data.subarray(1, 17));
  } catch {
    return null;
  }
  const index = new DataView(data.buffer, data.byteOffset, data.byteLength).getUint32(17);
  return { id, index, data: data.subarray(21) };
}

/** Random message id (base64url, 16 chars: also the chunk-id width). */
export function newId(): string {
  const b = crypto.getRandomValues(new Uint8Array(12));
  return btoa(String.fromCharCode(...b))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}
