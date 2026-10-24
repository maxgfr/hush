/**
 * Saving a download without holding it in memory: one file goes straight to
 * disk, several are streamed into a ZIP (stored, not compressed: the bytes are
 * already random-looking or will be checked as they are).
 *
 * Where the bytes go, by preference:
 *  - small downloads: memory, then an ordinary download (works everywhere);
 *  - the File System Access API (Chromium): a file the user picks;
 *  - this app's service worker, which serves a stream as an attachment
 *    (Firefox, Safari);
 *  - memory again, when neither exists.
 */
import { PREVIEWABLE } from "../app/files";
import type { DropFileInfo, DropManifest, DropSink } from "./seal";

/** Up to this size, a download is assembled in memory. */
export const IN_MEMORY = 256 * 1024 * 1024;

interface ByteWriter {
  write(data: Uint8Array): Promise<void>;
  close(): Promise<void>;
  abort(): Promise<void>;
}

/** The safe last component of a path, for a file name. */
export function baseName(path: string): string {
  const name = (path.split("/").pop() ?? "").replace(/^\.+/, "").trim();
  return name || "file";
}

// ── writers ─────────────────────────────────────────────────────────────────

function memoryWriter(name: string, mime: string, onUrl?: (url: string) => void): ByteWriter {
  let parts: Uint8Array[] = [];
  return {
    async write(d) {
      parts.push(d.slice());
    },
    async close() {
      // Only images the app previews keep their type: anything else opened from its blob URL
      // would render as a document of this origin.
      const type = PREVIEWABLE.test(mime) ? mime : "application/octet-stream";
      const url = URL.createObjectURL(new Blob(parts as BlobPart[], { type }));
      parts = [];
      const a = document.createElement("a");
      a.href = url;
      a.download = name;
      a.rel = "noopener";
      document.body.append(a);
      a.click();
      a.remove();
      if (onUrl) onUrl(url);
      else setTimeout(() => URL.revokeObjectURL(url), 60_000);
    },
    async abort() {
      parts = [];
    },
  };
}

interface FsWritable {
  write(d: Uint8Array): Promise<void>;
  close(): Promise<void>;
  abort(): Promise<void>;
}

type SavePicker = (o: {
  suggestedName: string;
}) => Promise<{ createWritable(): Promise<FsWritable> }>;

/** Must be called during the click: the picker needs a user gesture. */
async function fsWriter(name: string): Promise<ByteWriter | null> {
  const pick = (window as unknown as { showSaveFilePicker?: SavePicker }).showSaveFilePicker;
  if (!pick) return null;
  const handle = await pick({ suggestedName: name });
  const w = await handle.createWritable();
  return { write: (d) => w.write(d), close: () => w.close(), abort: () => w.abort() };
}

/** Streams through the service worker, which answers `./__drop/<id>` as an attachment. */
async function swWriter(name: string): Promise<ByteWriter | null> {
  const ctl = navigator.serviceWorker?.controller;
  if (!ctl) return null;
  const id = Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
  const ch = new MessageChannel();
  let credits = 0;
  let cancelled = false;
  let wake: (() => void) | null = null;
  const ready = new Promise<boolean>((resolve) => {
    const t = setTimeout(() => resolve(false), 3000);
    ch.port1.onmessage = (m) => {
      const d = m.data as { ready?: boolean; pull?: boolean; cancel?: boolean };
      if (d.ready) {
        clearTimeout(t);
        resolve(true);
      }
      if (d.pull) credits++;
      if (d.cancel) cancelled = true;
      wake?.();
    };
  });
  ctl.postMessage({ type: "drop-stream", id, name }, [ch.port2]);
  if (!(await ready)) return null;
  const a = document.createElement("a");
  a.href = `./__drop/${id}`;
  a.rel = "noopener";
  document.body.append(a);
  a.click();
  a.remove();
  const credit = async () => {
    while (credits === 0 && !cancelled) await new Promise<void>((r) => (wake = r));
    if (cancelled) throw new Error("the download was cancelled");
    credits--;
  };
  return {
    async write(d) {
      await credit();
      const copy = d.slice();
      ch.port1.postMessage({ chunk: copy.buffer }, [copy.buffer]);
    },
    async close() {
      await credit();
      ch.port1.postMessage({ done: true });
    },
    async abort() {
      ch.port1.postMessage({ error: true });
    },
  };
}

/** Picks where a download of `size` bytes named `name` goes. Call it from the click handler. */
export async function openWriter(name: string, size: number, mime: string): Promise<ByteWriter> {
  const q = typeof location !== "undefined" && location.hostname === "localhost";
  const forceMemory = q && new URLSearchParams(location.search).has("savememory");
  if (size <= IN_MEMORY || forceMemory) return memoryWriter(name, mime);
  try {
    const fs = await fsWriter(name);
    if (fs) return fs;
  } catch (e) {
    // The user closed the picker: that is an answer.
    if (e instanceof DOMException && e.name === "AbortError") throw new Error("cancelled");
  }
  return (await swWriter(name)) ?? memoryWriter(name, mime);
}

// ── sinks ───────────────────────────────────────────────────────────────────

/** One file, straight into a writer. */
function fileSink(w: ByteWriter): DropSink {
  return {
    begin: async () => {},
    write: (d) => w.write(d),
    end: async () => {},
    close: () => w.close(),
    abort: () => w.abort(),
  };
}

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(data: Uint8Array, crc = 0): number {
  let c = ~crc >>> 0;
  for (let i = 0; i < data.length; i++)
    c = (CRC_TABLE[(c ^ (data[i] as number)) & 0xff] as number) ^ (c >>> 8);
  return ~c >>> 0;
}

function dosTime(ms: number): [number, number] {
  const d = new Date(ms);
  const time = (d.getHours() << 11) | (d.getMinutes() << 5) | (d.getSeconds() >> 1);
  const date =
    ((Math.max(d.getFullYear(), 1980) - 1980) << 9) | ((d.getMonth() + 1) << 5) | d.getDate();
  return [time, date];
}

class Buf {
  private readonly b: Uint8Array;
  private readonly v: DataView;
  private at = 0;
  constructor(n: number) {
    this.b = new Uint8Array(n);
    this.v = new DataView(this.b.buffer);
  }
  u16(n: number): this {
    this.v.setUint16(this.at, n, true);
    this.at += 2;
    return this;
  }
  u32(n: number): this {
    this.v.setUint32(this.at, n >>> 0, true);
    this.at += 4;
    return this;
  }
  u64(n: number): this {
    this.v.setBigUint64(this.at, BigInt(n), true);
    this.at += 8;
    return this;
  }
  raw(d: Uint8Array): this {
    this.b.set(d, this.at);
    this.at += d.length;
    return this;
  }
  get bytes(): Uint8Array {
    return this.b.subarray(0, this.at);
  }
}

interface ZipEntry {
  name: Uint8Array;
  offset: number;
  size: number;
  crc: number;
}

/**
 * A ZIP written as a stream: stored entries, sizes and CRC-32 after each file (data
 * descriptors), ZIP64 records when the archive may pass 4 GiB.
 */
export function zipSink(
  w: ByteWriter,
  manifest: Pick<DropManifest, "files" | "total">,
  now = Date.now(),
): DropSink {
  const enc = new TextEncoder();
  const overhead = manifest.files.reduce(
    (n, f) => n + 2 * enc.encode(f.path).length + 30 + 46 + 24 + 28,
    98,
  );
  const z64 = manifest.total + overhead >= 0xffffffff;
  const [time, date] = dosTime(now);
  const done: ZipEntry[] = [];
  let at = 0;
  let cur: ZipEntry | null = null;
  const out = async (d: Uint8Array) => {
    at += d.length;
    await w.write(d);
  };
  const FLAGS = 0x0808; // data descriptor, UTF-8 names
  return {
    async begin(_i: number, f: DropFileInfo) {
      const name = enc.encode(f.path);
      cur = { name, offset: at, size: 0, crc: 0 };
      const h = new Buf(30 + name.length + 20)
        .u32(0x04034b50)
        .u16(z64 ? 45 : 20)
        .u16(FLAGS)
        .u16(0)
        .u16(time)
        .u16(date)
        .u32(0)
        .u32(z64 ? 0xffffffff : 0)
        .u32(z64 ? 0xffffffff : 0)
        .u16(name.length)
        .u16(z64 ? 20 : 0)
        .raw(name);
      if (z64) h.u16(1).u16(16).u64(0).u64(0);
      await out(h.bytes);
    },
    async write(d) {
      if (!cur) return;
      cur.crc = crc32(d, cur.crc);
      cur.size += d.length;
      await out(d);
    },
    async end() {
      if (!cur) return;
      const dd = new Buf(24).u32(0x08074b50).u32(cur.crc);
      if (z64) dd.u64(cur.size).u64(cur.size);
      else dd.u32(cur.size).u32(cur.size);
      await out(dd.bytes);
      done.push(cur);
      cur = null;
    },
    async close() {
      const start = at;
      for (const e of done) {
        const c = new Buf(46 + e.name.length + 28)
          .u32(0x02014b50)
          .u16(z64 ? 45 : 20)
          .u16(z64 ? 45 : 20)
          .u16(FLAGS)
          .u16(0)
          .u16(time)
          .u16(date)
          .u32(e.crc)
          .u32(z64 ? 0xffffffff : e.size)
          .u32(z64 ? 0xffffffff : e.size)
          .u16(e.name.length)
          .u16(z64 ? 28 : 0)
          .u16(0)
          .u16(0)
          .u16(0)
          .u32(0)
          .u32(z64 ? 0xffffffff : e.offset)
          .raw(e.name);
        if (z64) c.u16(1).u16(24).u64(e.size).u64(e.size).u64(e.offset);
        await out(c.bytes);
      }
      const size = at - start;
      const n = done.length;
      if (z64) {
        const rec = at;
        await out(
          new Buf(56)
            .u32(0x06064b50)
            .u64(44)
            .u16(45)
            .u16(45)
            .u32(0)
            .u32(0)
            .u64(n)
            .u64(n)
            .u64(size)
            .u64(start).bytes,
        );
        await out(new Buf(20).u32(0x07064b50).u32(0).u64(rec).u32(1).bytes);
      }
      await out(
        new Buf(22)
          .u32(0x06054b50)
          .u16(0)
          .u16(0)
          .u16(z64 ? 0xffff : n)
          .u16(z64 ? 0xffff : n)
          .u32(z64 ? 0xffffffff : size)
          .u32(z64 ? 0xffffffff : start)
          .u16(0).bytes,
      );
      await w.close();
    },
    abort: () => w.abort(),
  };
}

/** The archive name for several files: their common folder, or a generic one. */
export function zipName(files: { path: string }[]): string {
  const tops = new Set(files.map((f) => f.path.split("/")[0]));
  const [top] = [...tops];
  return tops.size === 1 && top && files.every((f) => f.path.includes("/"))
    ? `${top}.zip`
    : "hush-files.zip";
}

/** A sink for a whole drop: the file itself, or a ZIP of all of them. Call it from the click. */
export async function openSink(manifest: Pick<DropManifest, "files" | "total">): Promise<DropSink> {
  const [only] = manifest.files;
  if (manifest.files.length === 1 && only) {
    return fileSink(await openWriter(baseName(only.path), only.size, only.mime));
  }
  const name = zipName(manifest.files);
  return zipSink(await openWriter(name, manifest.total, "application/zip"), manifest);
}
