/**
 * One authenticated peer: a ratcheting session from the core, carried over a
 * direct WebRTC link when it opens, or relayed (still end-to-end encrypted)
 * through the rendezvous transports when it does not — strict NATs, no TURN.
 */
import { signal } from "@preact/signals";
import { type Channel, Kind, Lane, openFrame, pollFrames, type Session } from "../core/wasm";
import { RtcLink } from "../net/peer";
import type { Rendezvous } from "../net/rendezvous";
import { iceServers, settings } from "./settings";

export type PeerState = "connecting" | "direct" | "relay" | "closed";

/** After this long without a direct link, frames go through the relays. */
const DIRECT_TIMEOUT_MS = 20_000;
const POLL_MS = 10_000;

export interface PeerHandlers {
  onMessage(peer: PeerConn, lane: number, data: Uint8Array): void;
  onClosed(peer: PeerConn, byPeer: boolean): void;
  onState?(peer: PeerConn): void;
}

export class PeerConn {
  readonly name = signal("");
  readonly state = signal<PeerState>("connecting");
  /** Both sides confirmed the emojis. */
  readonly verifiedByMe = signal(false);
  readonly verifiedByPeer = signal(false);
  rtc: RtcLink | null = null;
  private readonly unlisten: () => void;
  private readonly signal: Channel;
  private readonly timers: ReturnType<typeof setTimeout>[] = [];
  private closed = false;
  /**
   * Frames sealed before any path worked, sent once one does. Relayed right away, they could
   * reach a peer not yet listening on the session's channel (it finishes the handshake last):
   * a contact card sent then was lost.
   */
  private readonly queued: { lane: number; frame: Uint8Array }[] = [];

  constructor(
    readonly tag: Uint8Array,
    readonly session: Session,
    readonly hash: Uint8Array,
    readonly sas: Uint8Array | null,
    private readonly rendezvous: Rendezvous,
    private readonly handlers: PeerHandlers,
    name = "",
  ) {
    this.name.value = name;
    this.signal = session.channel();
    this.unlisten = rendezvous.listen(this.signal, (env) => {
      if (env.kind === Kind.SESSION) this.receive(env.body);
    });
    this.timers.push(
      setTimeout(() => {
        if (this.state.value === "connecting") this.setState("relay");
      }, DIRECT_TIMEOUT_MS),
    );
    const poll = setInterval(() => {
      for (const f of pollFrames(this.session, Date.now())) void this.sendFrame(f.lane, f.frame);
    }, POLL_MS);
    this.timers.push(poll as unknown as ReturnType<typeof setTimeout>);
  }

  /** A fresh WebRTC link with the user's ICE settings. */
  static newRtc(
    onOpen: () => void,
    onFrame: (lane: number, f: Uint8Array) => void,
    onClose: () => void,
  ): RtcLink {
    return new RtcLink(
      { iceServers: iceServers(), relayOnly: settings.value.relayOnly },
      { onOpen, onFrame, onClose },
    );
  }

  /** Adopts the WebRTC link negotiated during the handshake. */
  attach(rtc: RtcLink): void {
    this.rtc = rtc;
    if (rtc.isOpen) this.setState("direct");
  }

  rtcOpened(): void {
    if (!this.closed) this.setState("direct");
  }

  rtcClosed(): void {
    if (!this.closed && this.state.value === "direct") this.setState("relay");
  }

  get isVerified(): boolean {
    return this.verifiedByMe.value && this.verifiedByPeer.value;
  }

  private setState(s: PeerState): void {
    if (this.state.value === s) return;
    this.state.value = s;
    if (s === "direct" || s === "relay")
      for (const q of this.queued.splice(0)) void this.sendFrame(q.lane, q.frame);
    this.handlers.onState?.(this);
  }

  /** Seals and sends an application message. File chunks need the direct link. */
  async send(lane: number, data: Uint8Array): Promise<void> {
    if (this.closed) throw new Error("closed");
    if (lane === Lane.FILE && !this.rtc?.isOpen) throw new Error("direct link required");
    const frame = this.session.seal(lane, data);
    await this.sendFrame(lane, frame);
  }

  private async sendFrame(lane: number, frame: Uint8Array, now = false): Promise<void> {
    if (!now && this.state.value === "connecting" && this.queued.length < 256) {
      this.queued.push({ lane, frame });
      return;
    }
    if (this.rtc?.isOpen) {
      try {
        await this.rtc.send(lane, frame);
        return;
      } catch {
        // The link just dropped: fall through to the relays.
      }
    }
    if (lane === Lane.FILE) return;
    this.rendezvous.send(this.signal, Kind.SESSION, frame);
  }

  /** A sealed frame from the direct link or the relays. */
  receive(frame: Uint8Array): void {
    if (this.closed) return;
    let opened: ReturnType<typeof openFrame>;
    try {
      opened = openFrame(this.session, frame, Date.now());
    } catch {
      return; // forged, replayed or out of window: dropped silently
    }
    for (const r of opened.replies) void this.sendFrame(r.lane, r.frame);
    for (const m of opened.messages) this.handlers.onMessage(this, m.lane, m.data);
    if (opened.closed) this.shutdown(true);
  }

  /** Ends the session: tells the peer, erases the keys, closes the links. */
  end(): void {
    if (this.closed) return;
    const bye = this.session.close();
    if (bye) void this.sendFrame(Lane.CTRL, bye, true);
    // Give the farewell a moment to leave before tearing the link down.
    setTimeout(() => this.shutdown(false), 300);
    this.closed = true;
  }

  private shutdown(byPeer: boolean): void {
    this.closed = true;
    for (const t of this.timers) clearTimeout(t);
    this.unlisten();
    this.rtc?.close();
    this.setState("closed");
    try {
      this.session.free();
      this.signal.free();
    } catch {
      // already freed
    }
    this.handlers.onClosed(this, byPeer);
  }
}
