/**
 * What every conversation shares, however its peers were authenticated (a
 * link handshake, a contact handshake, an identifier request): the peers,
 * the messages, files, disappearing timers, verification and the ending.
 *
 * Messages only live in memory, unless a saved contact has history turned on
 * (then contacts.ts copies them into the encrypted vault).
 */
import { batch, computed, signal } from "@preact/signals";
import { type EstablishedResult, Lane } from "../core/wasm";
import { equalBytes, fromB64u, toB64u, toHex } from "../lib/bytes";
import type { RtcLink } from "../net/peer";
import type { Rendezvous } from "../net/rendezvous";
import { t } from "../ui/i18n";
import {
  AUTO_DOWNLOAD,
  Download,
  type FileItem,
  fileOffer,
  hashFile,
  newFileItem,
  streamFile,
} from "./files";
import { PeerConn } from "./peer";
import {
  type ChatMessage,
  type CtrlMessage,
  decodeChat,
  decodeChunk,
  decodeCtrl,
  encodeJson,
  MAX_TEXT,
  newId,
} from "./protocol";

export type ChatStatus =
  | "waiting" // nobody else yet
  | "connecting" // a handshake is under way
  | "open" // at least one peer
  | "taken" // a one-to-one link someone else already opened
  | "expired"
  | "ended" // we ended it
  | "left"; // the other person ended it (one-to-one)

export interface ChatItem {
  id: string;
  /** null = us, undefined = system notice. */
  from: PeerConn | null | undefined;
  author: string;
  ts: number;
  text?: string;
  file?: FileItem;
  expiresAt?: number;
}

export interface ChatHooks {
  /** A peer sent its contact card (contacts.ts verifies and stores it). */
  onCard?(chat: Chat, peer: PeerConn, card: Uint8Array): void;
  /** A text message was shown (contacts.ts keeps history when asked to). */
  onText?(chat: Chat, item: ChatItem): void;
}

export interface Pending {
  rtc: RtcLink;
  bind: (p: PeerConn) => void;
}

export abstract class Chat {
  readonly status = signal<ChatStatus>("waiting");
  readonly peers = signal<PeerConn[]>([]);
  readonly items = signal<ChatItem[]>([]);
  readonly ttl = signal(0);
  readonly locked = signal(false);
  readonly unread = signal(0);
  readonly connected = computed(() => this.peers.value.some((p) => p.state.value !== "closed"));
  abstract readonly group: boolean;

  private readonly outgoing = new Map<string, File>();
  private readonly downloads = new Map<string, Download>();
  /** Downloads asked for before the direct link opened (the offer may come through the relays). */
  private readonly wanted = new Set<FileItem>();
  protected readonly timers: ReturnType<typeof setInterval>[] = [];
  protected finished = false;

  constructor(
    protected readonly rendezvous: Rendezvous,
    protected readonly myName: () => string,
    protected readonly hooks: ChatHooks,
  ) {
    this.timers.push(setInterval(() => this.sweep(), 1000));
  }

  get isFinished(): boolean {
    return this.finished;
  }

  protected hello(extra: Record<string, unknown> = {}): Uint8Array {
    return encodeJson({ v: 1, name: this.myName(), ...extra });
  }

  /** A WebRTC link whose frames are queued until the session exists. */
  protected newRtc(): Pending {
    let peer: PeerConn | null = null;
    const early: Uint8Array[] = [];
    const rtc = PeerConn.newRtc(
      () => peer?.rtcOpened(),
      (_lane, frame) => (peer ? peer.receive(frame) : early.push(frame)),
      () => peer?.rtcClosed(),
    );
    return {
      rtc,
      bind: (p) => {
        peer = p;
        p.attach(rtc);
        for (const f of early.splice(0)) p.receive(f);
      },
    };
  }

  protected addPeer(
    tag: Uint8Array,
    res: EstablishedResult,
    name: string,
    pending: Pending,
  ): PeerConn | null {
    if (this.finished) {
      pending.rtc.close();
      res.session.free();
      return null;
    }
    const peer = new PeerConn(
      tag,
      res.session,
      res.hash,
      res.sas,
      this.rendezvous,
      {
        onMessage: (p, lane, data) => this.onMessage(p, lane, data),
        onClosed: (p, byPeer) => this.onPeerClosed(p, byPeer),
        onState: (p) => {
          this.peers.value = [...this.peers.value];
          if (p.state.value !== "direct") return;
          for (const item of this.wanted) {
            if (item.owner !== p) continue;
            this.wanted.delete(item);
            this.download(item);
          }
        },
      },
      name,
    );
    pending.bind(peer);
    batch(() => {
      this.peers.value = [...this.peers.value, peer];
      this.status.value = "open";
    });
    if (this.group) this.notice(`${peer.name.value || t("someone")} ${t("joinedSuffix")}`);
    void peer.send(Lane.CTRL, encodeJson({ t: "name", name: this.myName() })).catch(() => {});
    if (this.locked.value)
      void peer.send(Lane.CTRL, encodeJson({ t: "lock", locked: true })).catch(() => {});
    return peer;
  }

  protected onPeerClosed(peer: PeerConn, byPeer: boolean): void {
    if (this.finished) return;
    const rest = this.peers.value.filter((p) => p !== peer);
    this.peers.value = rest;
    if (!this.group) {
      if (byPeer) this.status.value = "left";
      this.finish();
      return;
    }
    this.notice(`${peer.name.value || t("someone")} ${t("leftSuffix")}`);
    if (rest.length === 0) this.onEmpty();
  }

  /** The last peer of a group left. */
  protected onEmpty(): void {}

  // ----------------------------------------------------------------------------------------
  // Incoming

  private onMessage(peer: PeerConn, lane: number, data: Uint8Array): void {
    if (lane === Lane.CTRL) {
      const m = decodeCtrl(data);
      if (m) this.onCtrl(peer, m);
    } else if (lane === Lane.CHAT) {
      const m = decodeChat(data);
      if (m) void this.onChat(peer, m);
    } else if (lane === Lane.FILE) {
      const c = decodeChunk(data);
      if (!c) return;
      const key = `${toHex(peer.tag)}/${c.id}`;
      const dl = this.downloads.get(key);
      if (!dl?.add(c.index, c.data)) return;
      this.downloads.delete(key);
      // Tell the sender it arrived intact (the CLI's `hush send` exits on it).
      if (dl.item.state.value === "done") {
        void peer.send(Lane.CHAT, encodeJson({ t: "file-end", id: c.id })).catch(() => {});
      }
    }
  }

  private onCtrl(peer: PeerConn, m: CtrlMessage): void {
    switch (m.t) {
      case "name":
        peer.name.value = m.name;
        this.items.value = this.items.value.map((i) =>
          i.from === peer ? { ...i, author: m.name } : i,
        );
        break;
      case "verified":
        peer.verifiedByPeer.value = true;
        break;
      case "lock":
        this.locked.value = m.locked;
        break;
      case "card":
        try {
          this.hooks.onCard?.(this, peer, fromB64u(m.card));
        } catch {
          // malformed card: ignored
        }
        break;
    }
  }

  private async onChat(peer: PeerConn, m: ChatMessage): Promise<void> {
    const now = Date.now();
    switch (m.t) {
      case "msg": {
        const item: ChatItem = {
          id: m.id,
          from: peer,
          author: peer.name.value,
          ts: now,
          text: m.text,
          ...(m.ttl ? { expiresAt: now + m.ttl * 1000 } : {}),
        };
        this.push(item);
        this.unread.value++;
        this.hooks.onText?.(this, item);
        break;
      }
      case "del":
        this.items.value = this.items.value.filter((i) => !(i.id === m.id && i.from === peer));
        break;
      case "file": {
        const item = newFileItem(m, peer);
        this.push({
          id: m.id,
          from: peer,
          author: peer.name.value,
          ts: now,
          file: item,
          ...(m.ttl ? { expiresAt: now + m.ttl * 1000 } : {}),
        });
        this.unread.value++;
        if (m.mime.startsWith("image/") && m.size <= AUTO_DOWNLOAD && peer.rtc?.isOpen)
          this.download(item);
        break;
      }
      case "file-get": {
        const file = this.outgoing.get(m.id);
        if (file) await streamFile(file, m.id, peer).catch(() => {});
        break;
      }
      case "file-cancel":
        this.downloads.get(`${toHex(peer.tag)}/${m.id}`)?.fail();
        break;
    }
  }

  protected push(item: ChatItem): void {
    this.items.value = [...this.items.value, item];
  }

  protected notice(text: string): void {
    this.push({ id: newId(), from: undefined, author: "", ts: Date.now(), text });
  }

  protected live(): PeerConn[] {
    return this.peers.value.filter((p) => p.state.value !== "closed");
  }

  // ----------------------------------------------------------------------------------------
  // Outgoing

  async sendText(text: string): Promise<void> {
    const body = text.trim().slice(0, MAX_TEXT);
    if (!body || this.finished) return;
    const id = newId();
    const ts = Date.now();
    const ttl = this.ttl.value;
    const data = encodeJson({ t: "msg", id, ts, text: body, ...(ttl ? { ttl } : {}) });
    const item: ChatItem = {
      id,
      from: null,
      author: this.myName(),
      ts,
      text: body,
      ...(ttl ? { expiresAt: ts + ttl * 1000 } : {}),
    };
    this.push(item);
    this.hooks.onText?.(this, item);
    await Promise.all(this.live().map((p) => p.send(Lane.CHAT, data).catch(() => {})));
  }

  async sendFile(file: File): Promise<void> {
    if (this.finished) return;
    const id = newId();
    const ts = Date.now();
    const ttl = this.ttl.value;
    const item = newFileItem(
      {
        id,
        name: file.name || "file",
        size: file.size,
        mime: file.type || "application/octet-stream",
        sha256: "",
      },
      null,
    );
    item.state.value = "sending";
    this.push({
      id,
      from: null,
      author: this.myName(),
      ts,
      file: item,
      ...(ttl ? { expiresAt: ts + ttl * 1000 } : {}),
    });
    item.sha256 = await hashFile(file);
    item.url.value = URL.createObjectURL(file);
    item.state.value = "sent";
    this.outgoing.set(id, file);
    const offer = fileOffer(item, ts, ttl || undefined);
    await Promise.all(this.live().map((p) => p.send(Lane.CHAT, offer).catch(() => {})));
  }

  download(item: FileItem): void {
    const peer = item.owner;
    if (!peer || item.state.value === "downloading" || item.state.value === "done") return;
    if (!peer.rtc?.isOpen) {
      if (peer.state.value === "relay") item.state.value = "failed";
      else this.wanted.add(item);
      return;
    }
    const dl = new Download(item);
    if (item.size === 0) {
      dl.add(0, new Uint8Array(0));
      return;
    }
    this.downloads.set(`${toHex(peer.tag)}/${item.id}`, dl);
    void peer.send(Lane.CHAT, encodeJson({ t: "file-get", id: item.id })).catch(() => dl.fail());
  }

  /** "The emojis match" (or not: then that session is ended at once). */
  verify(peer: PeerConn, match: boolean): void {
    if (!match) {
      this.notice(t("mismatchClosed"));
      peer.end();
      return;
    }
    peer.verifiedByMe.value = true;
    void peer.send(Lane.CTRL, encodeJson({ t: "verified" })).catch(() => {});
  }

  sendCard(peer: PeerConn, card: Uint8Array): Promise<void> {
    return peer.send(Lane.CTRL, encodeJson({ t: "card", card: toB64u(card) }));
  }

  setTtl(seconds: number): void {
    this.ttl.value = seconds;
  }

  setLocked(locked: boolean): void {
    this.locked.value = locked;
    const data = encodeJson({ t: "lock", locked });
    for (const p of this.live()) void p.send(Lane.CTRL, data).catch(() => {});
  }

  /** Deletes one of our messages on every side. */
  async retract(id: string): Promise<void> {
    this.items.value = this.items.value.filter((i) => !(i.id === id && i.from === null));
    const data = encodeJson({ t: "del", id });
    await Promise.all(this.live().map((p) => p.send(Lane.CHAT, data).catch(() => {})));
  }

  protected sweep(): void {
    const now = Date.now();
    const items = this.items.value;
    if (!items.some((i) => i.expiresAt && i.expiresAt <= now)) return;
    for (const i of items) if (i.expiresAt && i.expiresAt <= now) revoke(i);
    this.items.value = items.filter((i) => !i.expiresAt || i.expiresAt > now);
  }

  /** Ends the conversation for everyone: erases keys, messages and files. */
  end(): void {
    if (this.finished) return;
    for (const p of this.live()) p.end();
    this.status.value = "ended";
    this.finish();
  }

  protected finish(): void {
    if (this.finished) return;
    this.finished = true;
    for (const t of this.timers) clearInterval(t);
    for (const i of this.items.value) revoke(i);
    this.items.value = [];
    this.outgoing.clear();
    this.downloads.clear();
    this.cleanup();
  }

  /** Subclasses release their handshake state and listeners. */
  protected abstract cleanup(): void;

  hasPeer(tag: Uint8Array): boolean {
    return this.peers.value.some((p) => equalBytes(p.tag, tag));
  }
}

function revoke(i: ChatItem): void {
  const url = i.file?.url.value;
  if (url) URL.revokeObjectURL(url);
}
