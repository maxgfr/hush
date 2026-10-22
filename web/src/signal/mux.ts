/**
 * Runs every enabled transport in parallel: an envelope is broadcast on all
 * of them and the first copy to arrive wins (duplicates are dropped). The
 * Rendezvous layer on top (../net/rendezvous.ts) seals and opens envelopes
 * with the core and follows the daily topic rotation.
 */
import { ManualTransport } from "./manual";
import { DEFAULT_RELAYS, NostrTransport } from "./nostr";
import { DEFAULT_TRACKERS, TorrentTransport } from "./torrent";
import type { SignalTransport, TransportHandlers, TransportName, TransportStatus } from "./types";

export interface SignalConfig {
  nostr: { enabled: boolean; relays: string[] };
  torrent: { enabled: boolean; trackers: string[] };
  manual: { enabled: boolean };
}

export const DEFAULT_SIGNAL_CONFIG: SignalConfig = {
  nostr: { enabled: true, relays: DEFAULT_RELAYS },
  torrent: { enabled: true, trackers: DEFAULT_TRACKERS },
  manual: { enabled: true },
};

export type TransportFactory = (
  name: TransportName,
  cfg: SignalConfig,
  peerId: string,
  handlers: TransportHandlers,
  onManualOutgoing: (code: string, to?: string) => void,
) => SignalTransport | null;

export const defaultTransportFactory: TransportFactory = (
  name,
  cfg,
  peerId,
  handlers,
  onManual,
) => {
  if (name === "nostr" && cfg.nostr.enabled && cfg.nostr.relays.length > 0) {
    return new NostrTransport(cfg.nostr.relays, handlers);
  }
  if (name === "torrent" && cfg.torrent.enabled && cfg.torrent.trackers.length > 0) {
    return new TorrentTransport(cfg.torrent.trackers, peerId, handlers);
  }
  if (name === "manual" && cfg.manual.enabled) return new ManualTransport(handlers, onManual);
  return null;
};

const NETWORK: readonly TransportName[] = ["nostr", "torrent"];
const DEDUP_MS = 5 * 60_000;

export interface MuxHandlers {
  onEnvelope(wire: string, topic: Uint8Array, via: TransportName): void;
  onStatus(statuses: TransportStatus[]): void;
  onManualOutgoing(code: string, to?: string): void;
}

export class SignalMux {
  readonly transports = new Map<TransportName, SignalTransport>();
  private readonly statuses = new Map<TransportName, TransportStatus>();
  private readonly seen = new Map<string, number>();

  constructor(
    cfg: SignalConfig,
    peerId: string,
    private readonly handlers: MuxHandlers,
    factory = defaultTransportFactory,
  ) {
    for (const name of ["nostr", "torrent", "manual"] as const) {
      const t = factory(
        name,
        cfg,
        peerId,
        {
          onEnvelope: (wire, topic) => this.deliver(wire, topic, name),
          onStatus: (s) => {
            this.statuses.set(name, s);
            this.handlers.onStatus(this.status());
          },
        },
        (code, to) => this.handlers.onManualOutgoing(code, to),
      );
      if (t) this.transports.set(name, t);
    }
  }

  private deliver(wire: string, topic: Uint8Array, via: TransportName): void {
    // The sealed part starts with a random nonce, so its prefix identifies the envelope.
    const key = `${topic[0]}:${topic[1]}:${wire.slice(0, 60)}`;
    const now = Date.now();
    if (this.seen.has(key)) return;
    this.seen.set(key, now);
    if (this.seen.size > 2048)
      for (const [k, t] of this.seen) if (now - t > DEDUP_MS) this.seen.delete(k);
    this.handlers.onEnvelope(wire, topic, via);
  }

  join(topic: Uint8Array): void {
    for (const t of this.transports.values()) t.join(topic);
  }

  leave(topic: Uint8Array): void {
    for (const t of this.transports.values()) t.leave(topic);
  }

  /** Sends on the network transports, plus any explicitly requested in `via` (e.g. manual). */
  send(
    topic: Uint8Array,
    wire: string,
    opts: { to?: string; via?: readonly TransportName[] } = {},
  ): void {
    const names = opts.via ?? NETWORK;
    for (const name of names) this.transports.get(name)?.send(topic, wire, opts.to);
  }

  manual(): ManualTransport | undefined {
    const t = this.transports.get("manual");
    return t instanceof ManualTransport ? t : undefined;
  }

  status(): TransportStatus[] {
    return [...this.transports.keys()].map(
      (name) => this.statuses.get(name) ?? { name, state: "connecting", endpoints: [] },
    );
  }

  close(): void {
    for (const t of this.transports.values()) t.close();
    this.transports.clear();
  }
}
