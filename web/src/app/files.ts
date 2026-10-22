/**
 * File transfer over the file lane: the sender offers (name, size, SHA-256),
 * the receiver asks, chunks stream in order over the ordered DataChannel and
 * the result is checked against the announced hash before it is shown.
 */
import { sha256 } from "@noble/hashes/sha2.js";
import { type Signal, signal } from "@preact/signals";
import { Lane } from "../core/wasm";
import { toHex } from "../lib/bytes";
import type { PeerConn } from "./peer";
import { CHUNK_SIZE, encodeChunk, encodeJson } from "./protocol";

export type FileState = "offered" | "sending" | "downloading" | "done" | "failed" | "sent";

export interface FileItem {
  id: string;
  name: string;
  size: number;
  mime: string;
  sha256: string;
  state: Signal<FileState>;
  progress: Signal<number>;
  /** Object URL once available (ours, or verified download). */
  url: Signal<string | null>;
  /** Who offered it (null when it is ours). */
  owner: PeerConn | null;
}

/** Images up to this size are fetched without a click, to show a preview. */
export const AUTO_DOWNLOAD = 8 * 1024 * 1024;

export async function hashFile(file: Blob): Promise<string> {
  const h = sha256.create();
  const reader = file.stream().getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    h.update(value);
  }
  return toHex(h.digest());
}

export function newFileItem(
  meta: { id: string; name: string; size: number; mime: string; sha256: string },
  owner: PeerConn | null,
): FileItem {
  return {
    ...meta,
    owner,
    state: signal<FileState>(owner ? "offered" : "sent"),
    progress: signal(owner ? 0 : 1),
    url: signal<string | null>(null),
  };
}

/** Streams one of our files to a peer that asked for it. */
export async function streamFile(file: Blob, id: string, peer: PeerConn): Promise<void> {
  let index = 0;
  for (let off = 0; off < file.size; off += CHUNK_SIZE) {
    const chunk = new Uint8Array(await file.slice(off, off + CHUNK_SIZE).arrayBuffer());
    await peer.send(Lane.FILE, encodeChunk(id, index++, chunk));
  }
}

/** Image types shown inline (never SVG: it can carry script). */
export const PREVIEWABLE = /^image\/(png|jpeg|gif|webp|avif)$/;

/** Collects the chunks of a download and verifies the result. */
export class Download {
  private readonly parts: Uint8Array[] = [];
  private readonly hash = sha256.create();
  private received = 0;
  private next = 0;

  constructor(readonly item: FileItem) {
    item.state.value = "downloading";
  }

  /** Returns true once complete (verified or failed). */
  add(index: number, data: Uint8Array): boolean {
    if (index !== this.next || this.received + data.length > this.item.size) {
      this.fail();
      return true;
    }
    this.next++;
    this.parts.push(data.slice());
    this.hash.update(data);
    this.received += data.length;
    this.item.progress.value = this.item.size ? this.received / this.item.size : 1;
    if (this.received < this.item.size) return false;
    if (toHex(this.hash.digest()) !== this.item.sha256) {
      this.fail();
      return true;
    }
    // Only images the app previews keep the sender's type: anything else (HTML, SVG…) opened
    // from its blob URL would render as a document of this origin.
    const type = PREVIEWABLE.test(this.item.mime) ? this.item.mime : "application/octet-stream";
    const blob = new Blob(this.parts as BlobPart[], { type });
    this.parts.length = 0;
    this.item.url.value = URL.createObjectURL(blob);
    this.item.state.value = "done";
    return true;
  }

  fail(): void {
    this.parts.length = 0;
    this.item.state.value = "failed";
  }
}

export const fileOffer = (item: FileItem, ts: number, ttl?: number) =>
  encodeJson({
    t: "file",
    id: item.id,
    ts,
    name: item.name,
    size: item.size,
    mime: item.mime,
    sha256: item.sha256,
    ...(ttl ? { ttl } : {}),
  });
