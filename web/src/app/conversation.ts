/**
 * A conversation opened from a link: one-to-one (the first person to open
 * the link takes it) or a group (everyone holding the link joins a full mesh
 * of pairwise sessions, until the room is locked).
 *
 * Link handshake, as seen from here (crates/hush-core/src/handshake.rs):
 *   joiner  ──HS1 {name}────────────▶ every member (creator first)
 *   member  ──HS2 {name, sdp offer}─▶ joiner          (to: joiner's tag)
 *   joiner  ──HS3 {sdp answer}──────▶ member          → session + WebRTC
 *
 * Nothing is written to disk. "End" erases every key and the messages.
 */
import {
  type Channel,
  type EstablishedResult,
  hush,
  Kind,
  type LinkCreator,
  type LinkCreatorPending,
  type LinkJoiner,
  type LinkJoinerReady,
  type LinkSecrets,
  linkInfo,
  normalizePassphrase,
  type OpenedEnvelope,
} from "../core/wasm";
import { toHex } from "../lib/bytes";
import type { Rendezvous } from "../net/rendezvous";
import { Chat, type ChatHooks, type Pending } from "./chat";
import { decodeHello, encodeJson } from "./protocol";

export type { ChatItem, ChatStatus } from "./chat";

const RESEND_MS = 6000;
const GROUP_RESENDS = 5;
export const MAX_GROUP = 16;
/** Handshakes in flight at once, and how long one may wait for its last message. */
const MAX_PENDING = 4;
const PENDING_MS = 45_000;

export class Conversation extends Chat {
  readonly group: boolean;
  readonly expiresMs: number;

  private readonly channel: Channel;
  private readonly unlisten: () => void;
  private joiner: LinkJoiner | null = null;
  private hs1: Uint8Array | null = null;
  private readonly creating = new Map<string, Pending>();
  private readonly awaitingHs3 = new Map<
    string,
    { state: LinkCreator; name: string; rtc: Pending }
  >();
  private readonly answered = new Set<string>();
  /** HS2s we are answering (each opens a connection). */
  private joining = 0;

  private constructor(
    readonly fragment: string,
    private readonly secrets: LinkSecrets,
    readonly role: "creator" | "joiner",
    rendezvous: Rendezvous,
    myName: () => string,
    hooks: ChatHooks,
  ) {
    super(rendezvous, myName, hooks);
    this.group = secrets.group;
    this.expiresMs = secrets.expiresMs;
    this.channel = secrets.channel();
    this.unlisten = rendezvous.listen(this.channel, (env) => this.onEnvelope(env));
  }

  /** A fresh link: we are its creator and wait for the other side. */
  static create(
    rendezvous: Rendezvous,
    opts: { group: boolean; ttlMin: number; passphrase?: string; name: () => string; ttl?: number },
    hooks: ChatHooks = {},
  ): Conversation {
    const core = hush();
    const fragment = core.linkNew(opts.group, opts.ttlMin, !!opts.passphrase, Date.now());
    const pass = opts.passphrase ? normalizePassphrase(opts.passphrase) : undefined;
    const secrets = new core.LinkSecrets(fragment, pass);
    const c = new Conversation(fragment, secrets, "creator", rendezvous, opts.name, hooks);
    c.ttl.value = opts.ttl ?? 0;
    return c;
  }

  /** A link made elsewhere (a drop link): we are its creator, and wait for the other side. */
  static host(
    rendezvous: Rendezvous,
    fragment: string,
    opts: { passphrase?: string; name: () => string },
    hooks: ChatHooks = {},
  ): Conversation {
    const pass = opts.passphrase ? normalizePassphrase(opts.passphrase) : undefined;
    const secrets = new (hush().LinkSecrets)(fragment, pass);
    return new Conversation(fragment, secrets, "creator", rendezvous, opts.name, hooks);
  }

  /** Opens someone's link. Throws `Expired` or `Malformed`; slow with a passphrase (Argon2id). */
  static join(
    rendezvous: Rendezvous,
    input: string,
    opts: { passphrase?: string; name: () => string; ttl?: number },
    hooks: ChatHooks = {},
  ): Conversation {
    const info = linkInfo(input);
    if (info.expiresMs <= Date.now()) throw new Error("Expired");
    const core = hush();
    const pass = opts.passphrase ? normalizePassphrase(opts.passphrase) : undefined;
    const secrets = new core.LinkSecrets(info.fragment, pass);
    const c = new Conversation(info.fragment, secrets, "joiner", rendezvous, opts.name, hooks);
    c.ttl.value = opts.ttl ?? 0;
    c.startJoin();
    return c;
  }

  /** The link to share (group members can share it too). */
  url(base: string): string {
    return `${base.split("#")[0]}#${this.fragment}`;
  }

  private startJoin(): void {
    const res = this.secrets.joinerStart(this.hello()) as { state: LinkJoiner; hs1: Uint8Array };
    this.joiner = res.state;
    this.hs1 = res.hs1;
    let sends = 0;
    const send = () => {
      if (this.finished) return;
      const waiting = this.status.value === "waiting";
      if (this.group ? sends >= GROUP_RESENDS && !waiting : !waiting) return;
      if (Date.now() >= this.expiresMs && waiting) {
        this.status.value = "expired";
        this.finish();
        return;
      }
      sends++;
      if (this.hs1) this.rendezvous.send(this.channel, Kind.HS1, this.hs1);
    };
    send();
    this.timers.push(setInterval(send, RESEND_MS));
  }

  private onEnvelope(env: OpenedEnvelope): void {
    if (this.finished) return;
    const from = toHex(env.from);
    switch (env.kind) {
      case Kind.HS1:
        void this.onHs1(env, from);
        break;
      case Kind.HS2:
        if (env.to) void this.onHs2(env, from);
        break;
      case Kind.HS3:
        if (env.to) void this.onHs3(env, from);
        break;
      case Kind.TAKEN:
        if (env.to && !this.group && this.peers.value.length === 0) {
          this.status.value = "taken";
          this.finish();
        }
        break;
    }
  }

  private isMember(): boolean {
    return this.role === "creator" || this.peers.value.length > 0;
  }

  /** Someone opened the link: answer if we may. */
  private async onHs1(env: OpenedEnvelope, from: string): Promise<void> {
    if (!this.isMember() || this.answered.has(from) || this.hasPeer(env.from)) return;
    const full = this.group
      ? this.locked.value ||
        this.peers.value.length >= MAX_GROUP - 1 ||
        // Someone holding the link could otherwise open connections without end.
        this.creating.size + this.awaitingHs3.size >= MAX_PENDING
      : this.peers.value.length > 0 || this.awaitingHs3.size > 0 || this.creating.size > 0;
    if (full) {
      // A one-to-one link is taken: say so, so a late recipient learns the link leaked.
      if (!this.group)
        this.rendezvous.send(this.channel, Kind.TAKEN, new Uint8Array(0), { to: env.from });
      return;
    }
    let read: { state: LinkCreatorPending; payload: Uint8Array };
    try {
      read = this.secrets.creatorReadHs1(env.body) as typeof read;
    } catch {
      return; // not someone holding this link (or a wrong passphrase)
    }
    this.answered.add(from);
    const hello = decodeHello(read.payload);
    const pending = this.newRtc();
    this.creating.set(from, pending);
    if (this.status.value === "waiting") this.status.value = "connecting";
    let hs2: { state: LinkCreator; hs2: Uint8Array };
    try {
      const offer = await pending.rtc.createOffer();
      hs2 = read.state.writeHs2(this.hello({ sdp: offer })) as typeof hs2;
    } catch {
      pending.rtc.close();
      return;
    } finally {
      this.creating.delete(from);
    }
    if (this.finished) {
      pending.rtc.close();
      return;
    }
    const waiting = { state: hs2.state, name: hello?.name ?? "", rtc: pending };
    this.awaitingHs3.set(from, waiting);
    this.timers.push(
      setTimeout(() => {
        if (this.awaitingHs3.get(from) !== waiting) return;
        this.awaitingHs3.delete(from);
        pending.rtc.close();
      }, PENDING_MS),
    );
    this.rendezvous.send(this.channel, Kind.HS2, hs2.hs2, { to: env.from });
  }

  private async onHs3(env: OpenedEnvelope, from: string): Promise<void> {
    const waiting = this.awaitingHs3.get(from);
    if (!waiting) return;
    this.awaitingHs3.delete(from);
    let res: EstablishedResult & { payload: Uint8Array };
    try {
      res = waiting.state.readHs3(env.body, Date.now()) as typeof res;
    } catch {
      waiting.rtc.rtc.close();
      return;
    }
    const hello = decodeHello(res.payload);
    if (hello?.sdp) await waiting.rtc.rtc.acceptAnswer(hello.sdp).catch(() => {});
    this.addPeer(env.from, res, waiting.name, waiting.rtc);
  }

  /** A member answered our HS1. */
  private async onHs2(env: OpenedEnvelope, from: string): Promise<void> {
    if (!this.joiner || this.answered.has(from)) return;
    if (!this.group && this.peers.value.length > 0) return;
    if (this.peers.value.length + this.joining >= MAX_GROUP - 1 || this.joining >= MAX_PENDING)
      return;
    let read: { state: LinkJoinerReady; payload: Uint8Array };
    try {
      read = this.joiner.readHs2(env.body) as typeof read;
    } catch {
      return;
    }
    this.answered.add(from);
    const hello = decodeHello(read.payload);
    if (!hello?.sdp) return;
    if (this.status.value === "waiting") this.status.value = "connecting";
    const pending = this.newRtc();
    let answer: string;
    this.joining++;
    try {
      answer = await pending.rtc.acceptOffer(hello.sdp);
    } catch {
      pending.rtc.close();
      return;
    } finally {
      this.joining--;
    }
    const res = read.state.writeHs3(
      encodeJson({ v: 1, sdp: answer }),
      Date.now(),
    ) as EstablishedResult & {
      hs3: Uint8Array;
    };
    this.rendezvous.send(this.channel, Kind.HS3, res.hs3, { to: env.from });
    this.addPeer(env.from, res, hello.name ?? "", pending);
  }

  protected override onEmpty(): void {
    if (this.role === "joiner") this.status.value = "waiting";
  }

  protected override sweep(): void {
    super.sweep();
    if (
      this.status.value === "waiting" &&
      this.role === "creator" &&
      Date.now() >= this.expiresMs
    ) {
      this.status.value = "expired";
      this.finish();
    }
  }

  protected cleanup(): void {
    this.unlisten();
    for (const p of this.creating.values()) p.rtc.close();
    for (const w of this.awaitingHs3.values()) w.rtc.rtc.close();
    this.creating.clear();
    this.awaitingHs3.clear();
    this.hs1 = null;
    try {
      this.joiner?.free();
      this.channel.free();
      this.secrets.free();
    } catch {
      // already freed
    }
    this.joiner = null;
  }
}
