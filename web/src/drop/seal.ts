/**
 * Sealed drops in the browser (the format is crates/hush-core/src/drop.rs; the
 * CLI does the same in crates/hush-app/src/drops.rs). Sending reads the files
 * twice: once to learn the segment addresses and build the header, published
 * first, then again to upload each segment, checked against the manifest
 * before it leaves. Receiving fetches, authenticates and hands the stream to a
 * sink file by file, each file's SHA-256 checked too.
 */
import { sha256 } from "@noble/hashes/sha2.js";
import type { DropOpener, DropSealer, Identity } from "../core/wasm";
import { hush, normalizePassphrase } from "../core/wasm";
import { equalBytes } from "../lib/bytes";
import { COPIES, del, get, newDeleteKey, put, putAny } from "./blossom";

import type { DropEntry } from "./entries";

export type { DropEntry } from "./entries";

export interface Locator {
  header: Uint8Array;
  servers: string[];
}

export interface DropFileInfo {
  path: string;
  size: number;
  mime: string;
  sha256: Uint8Array;
}

export interface DropManifest {
  files: DropFileInfo[];
  createdMs: number;
  expiresMs: number;
  once: boolean;
  total: number;
  segments: Uint8Array;
  deleteKey: Uint8Array;
}

export type Progress = (done: number, total: number) => void;

export { MAX_FILES, MAX_TOTAL } from "./entries";

import { MAX_FILES, MAX_TOTAL } from "./entries";

const MAX_HEADER = 8 * 1024 * 1024;
const PARALLEL = 3;

const tick = () => new Promise((r) => setTimeout(r, 0));

/** Reads the files as one stream, hashing each one on the way. */
class StreamReader {
  private idx = 0;
  private off = 0;
  private hasher = sha256.create();
  readonly hashes: Uint8Array[] = [];

  constructor(private readonly entries: DropEntry[]) {}

  async read(len: number): Promise<Uint8Array> {
    const out = new Uint8Array(len);
    let at = 0;
    while (at < len) {
      const e = this.entries[this.idx];
      if (!e) throw new Error("a file changed while it was being sent");
      if (this.off === e.file.size) {
        this.next();
        continue;
      }
      const n = Math.min(len - at, e.file.size - this.off);
      const part = new Uint8Array(await e.file.slice(this.off, this.off + n).arrayBuffer());
      if (part.length !== n) throw new Error("a file changed while it was being sent");
      out.set(part, at);
      this.hasher.update(part);
      this.off += n;
      at += n;
    }
    return out;
  }

  private next(): void {
    this.hashes.push(this.hasher.digest());
    this.hasher = sha256.create();
    this.idx++;
    this.off = 0;
  }

  /** The hashes, once everything was read (empty files at the end included). */
  done(): Uint8Array[] {
    while (this.idx < this.entries.length) {
      const e = this.entries[this.idx];
      if (e && this.off !== e.file.size) throw new Error("a file changed while it was being sent");
      this.next();
    }
    return this.hashes;
  }
}

export interface Prepared {
  sealer: DropSealer;
  entries: DropEntry[];
  manifest: Omit<DropManifest, "total">;
  header: Uint8Array;
}

/** Who a drop is sealed to: a link (with its passphrase), identities, or nobody (a session key). */
export interface Recipients {
  link?: { fragment: string; passphrase?: string };
  keys?: { ek: Uint8Array; epoch: number }[];
  signer?: Identity;
}

/** Seals the files once to build the header; `progress` sees the bytes read. */
export async function prepare(
  entries: DropEntry[],
  to: Recipients,
  expiresMs: number,
  once: boolean,
  progress: Progress = () => {},
): Promise<Prepared> {
  const total = entries.reduce((n, e) => n + e.file.size, 0);
  if (entries.length === 0) throw new Error("nothing to send");
  if (entries.length > MAX_FILES) throw new Error(`at most ${MAX_FILES} files`);
  if (total > MAX_TOTAL) throw new Error("at most 4 GB");
  const core = hush();
  const sealer = new core.DropSealer(total);
  if (to.link) {
    const pass = to.link.passphrase ? normalizePassphrase(to.link.passphrase) : undefined;
    sealer.addLink(to.link.fragment, pass);
  }
  for (const k of to.keys ?? []) sealer.addKey(k.ek, k.epoch);
  const count = sealer.segmentCount;
  const segments = new Uint8Array(count * 32);
  const reader = new StreamReader(entries);
  let done = 0;
  for (let i = 0; i < count; i++) {
    const data = await reader.read(sealer.segmentLength(i));
    segments.set(sealer.segmentHash(i, data), i * 32);
    done += data.length;
    progress(done, total);
    await tick();
  }
  const hashes = reader.done();
  const now = Date.now();
  const manifest = {
    files: entries.map((e, i) => ({
      path: e.path,
      size: e.file.size,
      mime: e.file.type || "application/octet-stream",
      sha256: hashes[i] as Uint8Array,
    })),
    createdMs: now,
    expiresMs: Math.max(expiresMs, now + 60_000),
    once,
    segments,
    deleteKey: newDeleteKey(),
  };
  const header = to.signer ? sealer.headerSigned(manifest, to.signer) : sealer.header(manifest);
  return { sealer, entries, manifest, header };
}

/** Uploads the header to `COPIES` of `servers`: the locator names those that took it. */
export async function uploadHeader(p: Prepared, servers: string[]): Promise<Locator> {
  const hosts = await putAny(servers, COPIES, p.header, p.manifest.deleteKey);
  return { header: hush().sha256(p.header), servers: hosts };
}

/** Uploads every segment to every server of the locator (one copy is enough). */
export async function uploadSegments(
  p: Prepared,
  locator: Locator,
  progress: Progress = () => {},
  signal?: AbortSignal,
): Promise<void> {
  const { sealer, manifest } = p;
  const total = p.entries.reduce((n, e) => n + e.file.size, 0);
  const reader = new StreamReader(p.entries);
  const running = new Set<Promise<void>>();
  let failure: unknown = null;
  let done = 0;
  for (let i = 0; i < sealer.segmentCount; i++) {
    if (signal?.aborted) throw new Error("cancelled");
    if (failure) throw failure;
    const data = await reader.read(sealer.segmentLength(i));
    const ct = sealer.sealSegment(i, data);
    // Sealing again must give the bytes the manifest names, or a nonce would be reused.
    if (!equalBytes(hush().sha256(ct), manifest.segments.subarray(i * 32, i * 32 + 32)))
      throw new Error("a file changed while it was being sent: start again");
    const n = data.length;
    const job = (async () => {
      const results = await Promise.allSettled(
        locator.servers.map((h) => put(h, ct, manifest.deleteKey)),
      );
      if (!results.some((r) => r.status === "fulfilled")) {
        const why = results.find((r): r is PromiseRejectedResult => r.status === "rejected");
        throw why?.reason ?? new Error("upload failed");
      }
      done += n;
      progress(done, total);
    })().catch((e) => {
      failure = e;
    });
    running.add(job);
    void job.finally(() => running.delete(job));
    if (running.size >= PARALLEL) await Promise.race(running);
  }
  await Promise.all(running);
  if (failure) throw failure;
  progress(total, total);
}

export async function fetchHeader(locator: Locator, signal?: AbortSignal): Promise<Uint8Array> {
  return get(locator.servers, locator.header, MAX_HEADER, signal);
}

/** Deletes the header and every segment; resolves to whether every server confirmed. */
export async function deleteDrop(manifest: DropManifest, locator: Locator): Promise<boolean> {
  const want = locator.servers.length;
  const shas = [locator.header];
  for (let i = 0; i < manifest.segments.length; i += 32)
    shas.push(manifest.segments.subarray(i, i + 32));
  let all = true;
  for (const sha of shas)
    all = (await del(locator.servers, sha, manifest.deleteKey)) === want && all;
  return all;
}

/** Where downloaded files go, one after the other. */
export interface DropSink {
  begin(index: number, file: DropFileInfo): Promise<void>;
  write(data: Uint8Array): Promise<void>;
  end(): Promise<void>;
  close(): Promise<void>;
  abort(reason: string): Promise<void>;
}

export const readManifest = (o: DropOpener): DropManifest => o.manifest() as DropManifest;

/** Splits the stream into files for a sink, checking each file's SHA-256. */
export class Splitter {
  private idx = -1;
  private left = 0;
  private hasher = sha256.create();

  constructor(
    private readonly files: DropFileInfo[],
    private readonly sink: DropSink,
  ) {}

  private async open(): Promise<void> {
    for (;;) {
      this.idx++;
      const f = this.files[this.idx];
      if (!f) throw new Error("more data than the manifest says");
      await this.sink.begin(this.idx, f);
      this.left = f.size;
      this.hasher = sha256.create();
      if (f.size > 0) return;
      await this.close();
    }
  }

  private async close(): Promise<void> {
    const f = this.files[this.idx];
    if (!f || !equalBytes(this.hasher.digest(), f.sha256))
      throw new Error(`${f?.path ?? "?"}: SHA-256 mismatch`);
    await this.sink.end();
  }

  async write(data: Uint8Array): Promise<void> {
    let at = 0;
    while (at < data.length) {
      if (this.left === 0) await this.open();
      const n = Math.min(this.left, data.length - at);
      const part = data.subarray(at, at + n);
      this.hasher.update(part);
      await this.sink.write(part);
      this.left -= n;
      at += n;
      if (this.left === 0) await this.close();
    }
  }

  /** Empty files left at the end, then the sink. */
  async finish(): Promise<void> {
    if (this.left !== 0) throw new Error("the drop ended early");
    while (this.idx + 1 < this.files.length) {
      const f = this.files[this.idx + 1];
      if (f && f.size > 0) throw new Error("the drop ended early");
      this.idx++;
      if (f) {
        await this.sink.begin(this.idx, f);
        this.hasher = sha256.create();
        await this.close();
      }
    }
    await this.sink.close();
  }
}

/** Downloads a drop's sealed copy into a sink. */
export async function download(
  opener: DropOpener,
  locator: Locator,
  sink: DropSink,
  progress: Progress = () => {},
  signal?: AbortSignal,
): Promise<void> {
  const m = readManifest(opener);
  const split = new Splitter(m.files, sink);
  const count = opener.segmentCount;
  const segSize = hush().dropSegmentSize();
  const fetchOne = (i: number) =>
    get(locator.servers, m.segments.subarray(i * 32, i * 32 + 32), segSize + 16, signal);
  const ahead: Promise<Uint8Array>[] = [];
  for (let i = 0; i < Math.min(PARALLEL, count); i++) ahead.push(fetchOne(i));
  // Unhandled rejections of prefetched segments are handled when awaited.
  for (const a of ahead) a.catch(() => {});
  let done = 0;
  try {
    for (let i = 0; i < count; i++) {
      const ct = await (ahead.shift() as Promise<Uint8Array>);
      if (i + PARALLEL < count) {
        const next = fetchOne(i + PARALLEL);
        next.catch(() => {});
        ahead.push(next);
      }
      const pt = opener.openSegment(i, ct);
      await split.write(pt);
      done += pt.length;
      progress(done, m.total);
    }
    await split.finish();
  } catch (e) {
    await sink.abort(e instanceof Error ? e.message : String(e));
    throw e;
  }
}
