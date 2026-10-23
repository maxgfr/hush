import { expect, test } from "bun:test";
import { randomBytes } from "../lib/bytes";
import { DEFAULT_SIGNAL_CONFIG, MAX_SEEN, SignalMux, type TransportFactory } from "./mux";
import type { TransportHandlers } from "./types";

test("duplicates are dropped, and a flood of new envelopes is remembered up to a bound", () => {
  const feeds: TransportHandlers[] = [];
  const factory: TransportFactory = (name, _cfg, _id, handlers) => {
    if (name === "nostr") feeds.push(handlers);
    return null;
  };
  const got: string[] = [];
  const mux = new SignalMux(
    DEFAULT_SIGNAL_CONFIG,
    "p".repeat(20),
    { onEnvelope: (wire) => got.push(wire), onStatus() {}, onManualOutgoing() {} },
    factory,
  );
  const relay = feeds[0] as TransportHandlers;
  const topic = randomBytes(32);
  relay.onEnvelope("first", topic);
  relay.onEnvelope("first", topic);
  expect(got).toEqual(["first"]);
  for (let n = 0; n < MAX_SEEN * 2; n++) relay.onEnvelope(`e${n}`, topic);
  const seen = (mux as unknown as { seen: Map<string, number> }).seen;
  expect(seen.size).toBe(MAX_SEEN);
  // The newest are still recognized.
  relay.onEnvelope(`e${MAX_SEEN * 2 - 1}`, topic);
  expect(got).toHaveLength(1 + MAX_SEEN * 2);
});
