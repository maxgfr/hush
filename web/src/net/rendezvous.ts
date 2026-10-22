/**
 * Rendezvous: where two devices that share a secret (a link, a contact, an
 * identifier) find each other. Envelopes are sealed by the core and carried
 * by every enabled transport (Nostr relays, WebTorrent trackers, or by hand
 * as QR codes). Relays and trackers only see a rotating topic and ciphertext.
 *
 * Each listener follows its channel's daily topic (today and yesterday, so a
 * conversation survives midnight UTC).
 */
import { type Channel, hush, Kind, type OpenedEnvelope, openEnvelope } from "../core/wasm";
import { equalBytes, fromB64u, toB64u, toHex } from "../lib/bytes";
import { type SignalConfig, SignalMux, type TransportFactory } from "../signal/mux";
import type { TransportName, TransportStatus } from "../signal/types";

export type EnvelopeListener = (env: OpenedEnvelope, via: TransportName) => void;

interface Subscription {
  channel: Channel;
  listener: EnvelopeListener;
  topics: Map<string, Uint8Array>;
}

export interface RendezvousHandlers {
  onStatus?(statuses: TransportStatus[]): void;
  /** A manual (QR / copy-paste) code to show for an outgoing envelope. */
  onManualOutgoing?(code: string): void;
}

export interface RendezvousOptions {
  config: SignalConfig;
  factory?: TransportFactory;
  now?: () => number;
}

const DAY_MS = 86_400_000;
const EVERYWHERE: readonly TransportName[] = ["nostr", "torrent", "manual"];
const NETWORK: readonly TransportName[] = ["nostr", "torrent"];
/** What can sensibly be carried by hand: the three link-handshake messages. */
const BY_HAND = new Set<number>([Kind.HS1, Kind.HS2, Kind.HS3]);

export class Rendezvous {
  /** Our random tag for this app session: the `from` of everything we send. Never an identity. */
  readonly tag: Uint8Array;
  readonly mux: SignalMux;
  private readonly subs = new Set<Subscription>();
  private readonly byTopic = new Map<string, Set<Subscription>>();
  private readonly now: () => number;
  private rollover: ReturnType<typeof setTimeout> | undefined;

  constructor(opts: RendezvousOptions, handlers: RendezvousHandlers = {}) {
    this.now = opts.now ?? Date.now;
    this.tag = hush().randomTag();
    const trackerPeerId = toB64u(this.tag).slice(0, 20);
    this.mux = new SignalMux(
      opts.config,
      trackerPeerId,
      {
        onEnvelope: (wire, topic, via) => this.receive(wire, topic, via),
        onStatus: (s) => handlers.onStatus?.(s),
        onManualOutgoing: (code) => handlers.onManualOutgoing?.(code),
      },
      opts.factory,
    );
    this.scheduleRollover();
  }

  /** Starts delivering `channel`'s envelopes to `listener`; returns the unsubscribe function. */
  listen(channel: Channel, listener: EnvelopeListener): () => void {
    const sub: Subscription = { channel, listener, topics: new Map() };
    this.subs.add(sub);
    this.refresh(sub);
    return () => {
      if (!this.subs.delete(sub)) return;
      for (const [hex, topic] of sub.topics) this.unbind(sub, hex, topic);
    };
  }

  /** Seals `body` on today's topic of `channel` and sends it everywhere (or only `via`). */
  send(
    channel: Channel,
    kind: number,
    body: Uint8Array,
    opts: { to?: Uint8Array; via?: readonly TransportName[] } = {},
  ): void {
    const now = this.now();
    const topic = channel.topic(hush().utcDay(now));
    const sealed = channel.seal(topic, this.tag, opts.to ?? new Uint8Array(0), kind, body, now);
    // Link handshakes also become manual codes, shown if the user opens the offline panel.
    this.mux.send(topic, toB64u(sealed), { via: opts.via ?? (BY_HAND.has(kind) ? EVERYWHERE : NETWORK) });
  }

  /** Feeds a scanned or pasted manual code; false when it is not one. */
  ingestManual(code: string): boolean {
    return this.mux.manual()?.ingest(code) ?? false;
  }

  status(): TransportStatus[] {
    return this.mux.status();
  }

  close(): void {
    clearTimeout(this.rollover);
    this.subs.clear();
    this.byTopic.clear();
    this.mux.close();
  }

  private refresh(sub: Subscription): void {
    const day = hush().utcDay(this.now());
    const wanted = new Map<string, Uint8Array>();
    for (const d of [day, day - 1]) {
      const t = sub.channel.topic(d);
      wanted.set(toHex(t), t);
    }
    for (const [hex, topic] of sub.topics) if (!wanted.has(hex)) this.unbind(sub, hex, topic);
    for (const [hex, topic] of wanted) {
      if (sub.topics.has(hex)) continue;
      sub.topics.set(hex, topic);
      let set = this.byTopic.get(hex);
      if (!set) {
        set = new Set();
        this.byTopic.set(hex, set);
        this.mux.join(topic);
      }
      set.add(sub);
    }
  }

  private unbind(sub: Subscription, hex: string, topic: Uint8Array): void {
    sub.topics.delete(hex);
    const set = this.byTopic.get(hex);
    if (!set) return;
    set.delete(sub);
    if (set.size === 0) {
      this.byTopic.delete(hex);
      this.mux.leave(topic);
    }
  }

  private scheduleRollover(): void {
    const now = this.now();
    const next = (Math.floor(now / DAY_MS) + 1) * DAY_MS;
    this.rollover = setTimeout(
      () => {
        for (const sub of this.subs) this.refresh(sub);
        this.scheduleRollover();
      },
      Math.max(1000, next - now + 1000),
    );
  }

  private receive(wire: string, topic: Uint8Array, via: TransportName): void {
    const subs = this.byTopic.get(toHex(topic));
    if (!subs) return;
    let data: Uint8Array;
    try {
      data = fromB64u(wire);
    } catch {
      return;
    }
    // Several listeners may share one channel (e.g. a waiter for one reply): open once per
    // channel (the replay guard would refuse a second open) and deliver to all of them.
    const tried = new Set<Channel>();
    for (const sub of subs) {
      if (tried.has(sub.channel)) continue;
      tried.add(sub.channel);
      let env: OpenedEnvelope;
      try {
        env = openEnvelope(sub.channel, topic, data, this.now(), via === "manual");
      } catch {
        continue; // not for this channel, stale, or a duplicate from another transport
      }
      if (equalBytes(env.from, this.tag)) return;
      if (env.to && !equalBytes(env.to, this.tag)) return;
      for (const s of [...subs]) if (s.channel === sub.channel) s.listener(env, via);
      return;
    }
  }
}

export { Kind };
