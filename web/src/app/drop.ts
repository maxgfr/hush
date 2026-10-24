/**
 * Drops: files sent by link, like a conversation that only carries files.
 *
 * Sending seals the files once, publishes the header on blob servers and puts
 * where it is into the link, then uploads the segments. Meanwhile the page
 * keeps the link open as a one-to-one conversation: whoever opens it while we
 * are both online gets the files peer to peer. Receiving does both at once:
 * the sender, if they answer, otherwise the sealed copy.
 *
 * A drop sealed to people (contacts, or ourselves) has no link secret: only
 * their identities open it, and it carries our signature.
 */
import { effect, signal } from "@preact/signals";
import {
  type DropOpener,
  hush,
  normalizePassphrase,
  parseBundle,
  parseStoredBundle,
} from "../core/wasm";
import { cleanHost } from "../drop/blossom";
import { openSink } from "../drop/save";
import {
  type DropEntry,
  type DropManifest,
  deleteDrop,
  download,
  fetchHeader,
  type Locator,
  type Prepared,
  prepare,
  readManifest,
  uploadHeader,
  uploadSegments,
} from "../drop/seal";
import { fromB64u, fromHex } from "../lib/bytes";
import type { Chat } from "./chat";
import { Conversation } from "./conversation";
import type { FileItem } from "./files";
import { blossomServers } from "./settings";
import { myName, rendezvous } from "./state";
import { type Contact, ensureIdentity, identity } from "./store";

/** Peer to peer is used for the whole drop when it is this small, or when there is no copy. */
const P2P_MAX = 1024 ** 3;

export type DropTarget = { kind: "link" } | { kind: "people"; contacts: Contact[]; me: boolean };

export interface SendOptions {
  ttlMin: number;
  once: boolean;
  passphrase?: string;
  p2pOnly: boolean;
  target: DropTarget;
}

const base = () => location.href.split("#")[0]?.split("?")[0] ?? "";
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export class OutgoingDrop {
  readonly phase = signal<"sealing" | "uploading" | "ready" | "failed" | "cancelled">("sealing");
  readonly progress = signal(0);
  readonly url = signal<string | null>(null);
  /** The peer-to-peer side: nobody yet, someone receiving, delivered. */
  readonly peer = signal<"none" | "sending" | "delivered">("none");
  readonly error = signal<string | null>(null);
  readonly servers = signal<string[]>([]);
  /** The sealed copy was deleted from the servers (asked for, or a one-time drop delivered). */
  readonly copyDeleted = signal(false);
  readonly expiresMs: number;
  readonly total: number;
  /** Only the people it is sealed to can open it (no link secret). */
  readonly sealedToPeople: boolean;
  private conv: Conversation | null = null;
  private prepared: Prepared | null = null;
  private locator: Locator | null = null;
  private readonly abort = new AbortController();
  private stops: (() => void)[] = [];

  constructor(
    readonly entries: DropEntry[],
    readonly opts: SendOptions,
  ) {
    this.expiresMs = Date.now() + opts.ttlMin * 60_000;
    this.total = entries.reduce((n, e) => n + e.file.size, 0);
    this.sealedToPeople = opts.target.kind === "people";
  }

  async start(): Promise<void> {
    try {
      if (this.opts.target.kind === "people") await this.startSealed(this.opts.target);
      else await this.startLink();
    } catch (e) {
      if (this.phase.value === "cancelled") return;
      this.error.value = message(e);
      this.phase.value = "failed";
    }
  }

  private async startLink(): Promise<void> {
    const core = hush();
    const pass = this.opts.passphrase || undefined;
    const servers = this.opts.p2pOnly ? [] : blossomServers();
    let fragment = core.dropLinkNew(this.opts.ttlMin, !!pass, Date.now());
    if (servers.length) {
      this.prepared = await prepare(
        this.entries,
        { link: { fragment, ...(pass ? { passphrase: pass } : {}) } },
        this.expiresMs,
        this.opts.once,
        (d, t) => (this.progress.value = t ? d / t : 1),
      );
      this.locator = await uploadHeader(this.prepared, servers);
      this.servers.value = this.locator.servers;
      fragment = core.dropLinkWithLocator(fragment, this.locator.header, this.locator.servers);
    }
    if (this.phase.value === "cancelled") return;
    this.host(fragment, pass);
    this.url.value = `${base()}#${fragment}`;
    if (!this.prepared || !this.locator) {
      this.phase.value = "ready";
      return;
    }
    this.phase.value = "uploading";
    this.progress.value = 0;
    await uploadSegments(
      this.prepared,
      this.locator,
      (d, t) => (this.progress.value = t ? d / t : 1),
      this.abort.signal,
    );
    if (this.phase.value === "uploading") this.phase.value = "ready";
  }

  private async startSealed(target: { contacts: Contact[]; me: boolean }): Promise<void> {
    const core = hush();
    const servers = blossomServers();
    if (servers.length === 0) throw new Error("no blob server (Settings)");
    const me = await ensureIdentity(myName());
    const keys = target.contacts.map((c) => {
      const b = parseStoredBundle(fromB64u(c.bundle));
      return { ek: b.spk, epoch: b.spkEpoch };
    });
    if (target.me) {
      const b = parseBundle(me.bundle(myName(), Date.now()), Date.now());
      keys.push({ ek: b.spk, epoch: b.spkEpoch });
    }
    const once = this.opts.once && keys.length === 1 && !target.me;
    this.prepared = await prepare(
      this.entries,
      { keys, signer: me },
      this.expiresMs,
      once,
      (d, t) => {
        this.progress.value = t ? d / t : 1;
      },
    );
    this.locator = await uploadHeader(this.prepared, servers);
    this.servers.value = this.locator.servers;
    this.phase.value = "uploading";
    this.progress.value = 0;
    await uploadSegments(
      this.prepared,
      this.locator,
      (d, t) => (this.progress.value = t ? d / t : 1),
      this.abort.signal,
    );
    this.url.value = `${base()}#${core.sealedFragment(this.locator.header, this.locator.servers)}`;
    this.phase.value = "ready";
  }

  /** Keeps the link open as a conversation: whoever opens it now gets the files directly. */
  private host(fragment: string, passphrase?: string): void {
    const conv = Conversation.host(rendezvous(), fragment, {
      ...(passphrase ? { passphrase } : {}),
      name: myName,
    });
    this.conv = conv;
    let offered = false;
    this.stops.push(
      effect(() => {
        if (offered || conv.peers.value.length === 0) return;
        offered = true;
        this.peer.value = "sending";
        for (const e of this.entries) void conv.sendFile(e.file, e.path);
      }),
      effect(() => {
        const mine = conv.items.value.flatMap((i) => (i.from === null && i.file ? [i.file] : []));
        if (mine.length < this.entries.length || !mine.every((f) => f.delivered.value)) return;
        if (this.peer.value === "delivered") return;
        this.peer.value = "delivered";
        // A one-time drop that arrived has no reason to stay on the servers.
        if (this.opts.once) void this.deleteCopy();
      }),
    );
  }

  /** Stops the upload and deletes what is on the servers. */
  async deleteCopy(): Promise<boolean> {
    this.abort.abort();
    if (!this.prepared || !this.locator) return true;
    const manifest = { ...this.prepared.manifest, total: this.total };
    this.prepared = null;
    const all = await deleteDrop(manifest, this.locator).catch(() => false);
    this.copyDeleted.value = true;
    return all;
  }

  /** Cancels everything: the upload, the copy on the servers, the open link. */
  async cancel(): Promise<void> {
    this.phase.value = "cancelled";
    await this.deleteCopy();
    this.close();
  }

  close(): void {
    for (const s of this.stops) s();
    this.stops = [];
    this.conv?.end();
    this.conv = null;
  }
}

/** A file of an incoming drop, as the page shows it. */
export interface IncomingFile {
  path: string;
  size: number;
  mime: string;
}

export class IncomingDrop {
  readonly state = signal<"opening" | "ready" | "downloading" | "done" | "failed">("opening");
  readonly manifest = signal<DropManifest | null>(null);
  /** For a drop sealed to us: who signed it. */
  readonly sender = signal<string | null>(null);
  /** The sender answered the link, and a direct path is open. */
  readonly online = signal(false);
  readonly source = signal<"p2p" | "copy" | null>(null);
  readonly progress = signal(0);
  readonly error = signal<string | null>(null);
  /** The files the sender offers peer to peer (a drop without a sealed copy has only these). */
  readonly offers = signal<FileItem[]>([]);
  readonly deleted = signal(false);
  private opener: DropOpener | null = null;
  private conv: Conversation | null = null;
  private stops: (() => void)[] = [];

  private constructor(
    readonly locator: Locator | null,
    readonly expiresMs: number,
  ) {}

  /** A drop link (asks for its passphrase first when it has one). */
  static link(fragment: string, passphrase?: string): IncomingDrop {
    const core = hush();
    const info = core.linkInfo(fragment) as {
      expiresMs: number;
      fragment: string;
      locator?: { header: Uint8Array; servers: string[] };
    };
    const locator = info.locator
      ? {
          header: info.locator.header,
          servers: info.locator.servers.map(cleanHost).filter((h): h is string => !!h),
        }
      : null;
    const d = new IncomingDrop(locator, info.expiresMs);
    const pass = passphrase ? normalizePassphrase(passphrase) : undefined;
    d.join(info.fragment, pass);
    if (locator)
      void d.openCopy((header) => core.dropOpenLink(header, info.fragment, pass, Date.now()));
    else d.state.value = "ready";
    return d;
  }

  /** A drop sealed to identities: ours opens it. */
  static sealed(input: string): IncomingDrop {
    const core = hush();
    const info = core.sealedInfo(input) as { header: Uint8Array; servers: string[] };
    const locator = {
      header: info.header,
      servers: info.servers.map(cleanHost).filter((h): h is string => !!h),
    };
    const d = new IncomingDrop(locator, 0);
    void d.openCopy(async (header) => {
      // Only an identity we already have can be the one it was sealed to.
      const me = identity.value;
      if (!me) throw new Error("needIdentity");
      const o = core.dropOpenIdentity(header, me, Date.now());
      d.sender.value = o.sender ?? null;
      return o;
    });
    return d;
  }

  private async openCopy(
    open: (header: Uint8Array) => DropOpener | Promise<DropOpener>,
  ): Promise<void> {
    if (!this.locator) return;
    try {
      let header: Uint8Array;
      try {
        header = await fetchHeader(this.locator);
      } catch {
        throw new Error("dropGone");
      }
      this.opener = await open(header);
      this.manifest.value = readManifest(this.opener);
      if (this.state.value === "opening") this.state.value = "ready";
    } catch (e) {
      const m = message(e);
      const sealed = !this.conv;
      this.error.value =
        m === "Decrypt"
          ? sealed
            ? "notForYou"
            : "wrongPassphrase"
          : m === "Expired"
            ? "dropExpired"
            : m === "needIdentity" || m === "dropGone"
              ? m
              : "downloadFailed";
      // Without a copy, the sender may still be there.
      if (!this.conv) this.state.value = "failed";
      else if (this.state.value === "opening") this.state.value = "ready";
    }
  }

  /** The peer-to-peer side: the sender's page, if it is still open. */
  private join(fragment: string, passphrase?: string): void {
    let conv: Conversation;
    try {
      conv = Conversation.join(rendezvous(), fragment, {
        ...(passphrase ? { passphrase } : {}),
        name: myName,
      });
    } catch {
      return;
    }
    this.conv = conv;
    this.stops.push(
      effect(() => {
        this.offers.value = conv.items.value.flatMap((i) => (i.from && i.file ? [i.file] : []));
        this.online.value = conv.peers.value.some((p) => p.state.value === "direct");
      }),
    );
  }

  /** What the page lists: the manifest, or what the sender offers directly. */
  files(): IncomingFile[] {
    const m = this.manifest.value;
    if (m) return m.files.map((f) => ({ path: f.path, size: f.size, mime: f.mime }));
    return this.offers.value.map((f) => ({ path: f.path ?? f.name, size: f.size, mime: f.mime }));
  }

  private p2pReady(): boolean {
    const want = this.manifest.value?.files.length ?? this.offers.value.length;
    const total = this.files().reduce((n, f) => n + f.size, 0);
    return (
      this.online.value &&
      want > 0 &&
      this.offers.value.length >= want &&
      (!this.opener || total <= P2P_MAX)
    );
  }

  /** Saves everything (call it from the click: it may open a save dialog). */
  async save(): Promise<void> {
    const files = this.files();
    if (files.length === 0 || this.state.value === "downloading") return;
    const m = this.manifest.value;
    const total = files.reduce((n, f) => n + f.size, 0);
    const infos = m
      ? m.files
      : this.offers.value.map((f) => ({
          path: f.path ?? f.name,
          size: f.size,
          mime: f.mime,
          sha256: fromHex(f.sha256),
        }));
    let sink: Awaited<ReturnType<typeof openSink>>;
    try {
      sink = await openSink({ files: infos, total });
    } catch {
      return;
    }
    this.state.value = "downloading";
    this.progress.value = 0;
    this.error.value = null;
    try {
      if (this.conv && this.p2pReady()) {
        this.source.value = "p2p";
        await this.viaPeer(this.conv, sink, total);
      } else if (this.opener && this.locator) {
        this.source.value = "copy";
        await download(
          this.opener,
          this.locator,
          sink,
          (d, t) => (this.progress.value = t ? d / t : 1),
        );
      } else {
        throw new Error("senderGone");
      }
      this.state.value = "done";
      this.conv?.end();
      if (m?.once && this.locator)
        this.deleted.value = await deleteDrop(m, this.locator).catch(() => false);
    } catch (e) {
      await sink.abort(message(e)).catch(() => {});
      this.error.value = message(e) === "senderGone" ? "senderGone" : "downloadFailed";
      this.state.value = "failed";
    }
  }

  /** Peer to peer, one file after the other, in the manifest's order. */
  private async viaPeer(
    conv: Chat,
    sink: Awaited<ReturnType<typeof openSink>>,
    total: number,
  ): Promise<void> {
    const offers = [...this.offers.value];
    let before = 0;
    for (const [i, item] of offers.entries()) {
      conv.download(item);
      await new Promise<void>((resolve, reject) => {
        const stop = effect(() => {
          const s = item.state.value;
          this.progress.value = total ? (before + item.progress.value * item.size) / total : 1;
          if (s === "done")
            queueMicrotask(() => {
              stop();
              resolve();
            });
          if (s === "failed")
            queueMicrotask(() => {
              stop();
              reject(new Error("p2p"));
            });
        });
      });
      const blob = item.blob;
      if (!blob) throw new Error("p2p");
      await sink.begin(i, {
        path: item.path ?? item.name,
        size: item.size,
        mime: item.mime,
        sha256: fromHex(item.sha256),
      });
      const reader = blob.stream().getReader();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        await sink.write(value);
      }
      await sink.end();
      before += item.size;
    }
    await sink.close();
  }

  close(): void {
    for (const s of this.stops) s();
    this.stops = [];
    this.conv?.end();
    this.conv = null;
    try {
      this.opener?.free();
    } catch {
      // already freed
    }
    this.opener = null;
  }
}
