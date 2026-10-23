/** A WebSocket the test drives by hand: it opens at once and records what it is sent. */
import type { WebSocketCtor } from "../src/signal/types";

export interface FakeSocket {
  onopen: (() => void) | null;
  onmessage: ((e: { data: unknown }) => void) | null;
  readonly sent: string[];
}

export function fakeSockets(): { WS: WebSocketCtor; sockets: FakeSocket[] } {
  const sockets: FakeSocket[] = [];
  class FakeWS implements FakeSocket {
    onopen: (() => void) | null = null;
    onmessage: ((e: { data: unknown }) => void) | null = null;
    onclose: (() => void) | null = null;
    onerror: (() => void) | null = null;
    readonly sent: string[] = [];
    constructor() {
      sockets.push(this);
      queueMicrotask(() => this.onopen?.());
    }
    send(s: string): void {
      this.sent.push(s);
    }
    close(): void {}
  }
  return { WS: FakeWS as unknown as WebSocketCtor, sockets };
}
