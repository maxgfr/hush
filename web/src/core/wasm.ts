/**
 * Typed access to the Rust core compiled to WebAssembly. Every cryptographic
 * operation of the app goes through here; JavaScript never sees a key, only
 * handles (wasm classes), ciphertext and the plaintext it has to display.
 */
import init, * as core from "./pkg/hush_wasm.js";

export type {
  Channel,
  ContactInitiator,
  ContactInitiatorReady,
  ContactResponder,
  ContactResponderPending,
  Identity,
  LinkCreator,
  LinkCreatorPending,
  LinkJoiner,
  LinkJoinerReady,
  LinkSecrets,
  Session,
  Vault,
} from "./pkg/hush_wasm.js";

declare const __WASM_URL__: string;

let ready: Promise<typeof core> | null = null;

/** Loads the core once. In the browser from the hashed asset; in tests from disk. */
export function loadCore(): Promise<typeof core> {
  if (!ready) {
    ready = (async () => {
      if (typeof __WASM_URL__ === "string") {
        await init({ module_or_path: new URL(__WASM_URL__, location.href) });
      } else {
        const { readFileSync } = await import("node:fs");
        const path = new URL("./pkg/hush_wasm_bg.wasm", import.meta.url);
        core.initSync({ module: readFileSync(path) });
      }
      return core;
    })();
  }
  return ready;
}

/** The loaded core; only valid after `loadCore()` resolved. */
export function hush(): typeof core {
  return core;
}

/** Core error names, as thrown by every wasm call (`err.message`). */
export type CoreError =
  | "Malformed"
  | "Decrypt"
  | "BadSignature"
  | "IdMismatch"
  | "Expired"
  | "Replay"
  | "BadState"
  | "TooFarAhead"
  | "WeakProofOfWork"
  | "InvalidParameter"
  | "Outdated";

export function coreError(e: unknown): CoreError | null {
  const m = e instanceof Error ? e.message : String(e);
  return /^[A-Z][A-Za-z]+$/.test(m) ? (m as CoreError) : null;
}

// Shapes of the plain objects the core returns.

export interface LinkInfo {
  mode: "direct" | "group";
  expiresMs: number;
  passphrase: boolean;
  fragment: string;
}

export interface OpenedEnvelope {
  ts: number;
  from: Uint8Array;
  to: Uint8Array | null;
  kind: number;
  body: Uint8Array;
}

export interface OpenedFrame {
  messages: { lane: number; data: Uint8Array }[];
  replies: { lane: number; frame: Uint8Array }[];
  closed: boolean;
  rekeyed: boolean;
}

export interface EstablishedResult {
  session: core.Session;
  hash: Uint8Array;
  sas: Uint8Array | null;
}

export interface BundleInfo {
  id: string;
  idHash: Uint8Array;
  name: string;
  spkEpoch: number;
  spk: Uint8Array;
  expiresMs: number;
  bytes: Uint8Array;
}

/** Envelope kinds (crates/hush-core/src/envelope.rs). */
export const Kind = {
  HS1: 1,
  HS2: 2,
  HS3: 3,
  SESSION: 4,
  TAKEN: 5,
  HERE: 6,
  C1: 7,
  C2: 8,
  C3: 9,
  LOOKUP: 10,
  BUNDLE: 11,
  REQUEST: 12,
} as const;

/** Session lanes (crates/hush-core/src/session.rs). */
export const Lane = { CTRL: 0, CHAT: 1, FILE: 2, SIG: 3 } as const;

export const linkInfo = (input: string): LinkInfo => core.linkInfo(input) as LinkInfo;
export const openEnvelope = (
  ch: core.Channel,
  topic: Uint8Array,
  data: Uint8Array,
  now: number,
  manual: boolean,
): OpenedEnvelope => ch.open(topic, data, now, manual) as OpenedEnvelope;
export const openFrame = (s: core.Session, frame: Uint8Array, now: number): OpenedFrame =>
  s.open(frame, now) as OpenedFrame;
export const pollFrames = (s: core.Session, now: number): { lane: number; frame: Uint8Array }[] =>
  s.poll(now) as { lane: number; frame: Uint8Array }[];
export const sasEmojis = (indices: Uint8Array): [string, string][] =>
  core.sasEmojis(indices) as [string, string][];
export const verifyCard = (card: Uint8Array, hash: Uint8Array, now: number): BundleInfo =>
  core.verifyCard(card, hash, now) as BundleInfo;
export const parseBundle = (data: Uint8Array, now: number): BundleInfo =>
  core.parseBundle(data, now) as BundleInfo;
export const parseStoredBundle = (data: Uint8Array): BundleInfo =>
  core.parseStoredBundle(data) as BundleInfo;

/** Passphrases are NFKC-normalized here (the web build of the core leaves it to the host). */
export const normalizePassphrase = (p: string): string => p.normalize("NFKC");
