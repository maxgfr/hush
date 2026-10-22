/**
 * A WebRTC link to one peer: three negotiated DataChannels (ctrl, chat, file
 * — ids 0, 1, 2, so the CLI can open the same ones), non-trickle ICE (the
 * description is complete once gathering ends, so a single handshake message
 * carries it). The SDP only ever travels inside the PQ handshake.
 *
 * Everything sent here is already a sealed session frame: DTLS is a second
 * layer, not the one we rely on.
 */

export const CHANNELS = ["ctrl", "chat", "file"] as const;
export type ChannelLabel = (typeof CHANNELS)[number];

/** Above this, `send` waits for the buffer to drain (file transfers). */
const HIGH_WATER = 4 * 1024 * 1024;
const LOW_WATER = 1024 * 1024;

export interface PeerOptions {
  iceServers: RTCIceServer[];
  /** "relay" forces TURN, hiding our IP from the peer. */
  relayOnly?: boolean;
  gatherTimeoutMs?: number;
  rtc?: typeof RTCPeerConnection;
}

export interface PeerEvents {
  onOpen(): void;
  onFrame(lane: number, frame: Uint8Array): void;
  onClose(): void;
}

export class RtcLink {
  readonly pc: RTCPeerConnection;
  private readonly channels: RTCDataChannel[] = [];
  private opened = false;
  private closed = false;

  constructor(
    private readonly o: PeerOptions,
    private readonly events: PeerEvents,
  ) {
    const Ctor = o.rtc ?? RTCPeerConnection;
    this.pc = new Ctor({
      iceServers: o.iceServers,
      iceTransportPolicy: o.relayOnly ? "relay" : "all",
    });
    CHANNELS.forEach((label, id) => {
      const dc = this.pc.createDataChannel(label, { negotiated: true, id, ordered: true });
      dc.binaryType = "arraybuffer";
      dc.bufferedAmountLowThreshold = LOW_WATER;
      dc.onopen = () => this.checkOpen();
      dc.onclose = () => this.close();
      dc.onmessage = (ev: MessageEvent) => {
        if (ev.data instanceof ArrayBuffer) this.events.onFrame(id, new Uint8Array(ev.data));
      };
      this.channels.push(dc);
    });
    this.pc.addEventListener("connectionstatechange", () => {
      const s = this.pc.connectionState;
      if (s === "failed" || s === "closed") this.close();
    });
  }

  get isOpen(): boolean {
    return this.opened && !this.closed;
  }

  private checkOpen(): void {
    if (this.opened || this.closed) return;
    if (this.channels.every((c) => c.readyState === "open")) {
      this.opened = true;
      this.events.onOpen();
    }
  }

  private gathered(): Promise<void> {
    if (this.pc.iceGatheringState === "complete") return Promise.resolve();
    return new Promise((resolve) => {
      const done = () => {
        clearTimeout(timer);
        this.pc.removeEventListener("icegatheringstatechange", check);
        resolve();
      };
      const check = () => {
        if (this.pc.iceGatheringState === "complete") done();
      };
      const timer = setTimeout(done, this.o.gatherTimeoutMs ?? 4000);
      this.pc.addEventListener("icegatheringstatechange", check);
    });
  }

  /** Creator side: the full offer SDP. */
  async createOffer(): Promise<string> {
    await this.pc.setLocalDescription(await this.pc.createOffer());
    await this.gathered();
    return this.pc.localDescription?.sdp ?? "";
  }

  /** Joiner side: applies the offer, returns the full answer SDP. */
  async acceptOffer(sdp: string): Promise<string> {
    await this.pc.setRemoteDescription({ type: "offer", sdp });
    await this.pc.setLocalDescription(await this.pc.createAnswer());
    await this.gathered();
    return this.pc.localDescription?.sdp ?? "";
  }

  async acceptAnswer(sdp: string): Promise<void> {
    await this.pc.setRemoteDescription({ type: "answer", sdp });
  }

  /** Sends a sealed frame on its lane's channel, waiting while the buffer is full. */
  async send(lane: number, frame: Uint8Array): Promise<void> {
    const dc = this.channels[lane];
    if (dc?.readyState !== "open") throw new Error("channel closed");
    if (dc.bufferedAmount > HIGH_WATER) {
      await new Promise<void>((resolve) => {
        const t = setInterval(() => {
          if (dc.bufferedAmount <= LOW_WATER || dc.readyState !== "open") {
            clearInterval(t);
            resolve();
          }
        }, 20);
        dc.addEventListener(
          "bufferedamountlow",
          () => {
            clearInterval(t);
            resolve();
          },
          { once: true },
        );
      });
    }
    dc.send(frame as Uint8Array<ArrayBuffer>);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const c of this.channels) {
      try {
        c.close();
      } catch {
        // already closed
      }
    }
    try {
      this.pc.close();
    } catch {
      // already closed
    }
    this.events.onClose();
  }
}

/** Default STUN servers (editable in Settings); hush runs none of its own. */
export const DEFAULT_STUN = ["stun:stun.cloudflare.com:3478", "stun:stun.l.google.com:19302"];
