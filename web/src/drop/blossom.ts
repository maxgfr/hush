/**
 * Blossom blob servers (BUD-01, BUD-02), where sealed drops wait for their
 * recipients (crates/hush-app/src/blossom.rs is the same client). A blob is
 * addressed by the SHA-256 of its bytes and checked when it comes back;
 * uploads and deletions carry a kind-24242 event signed with the drop's
 * throwaway key. Servers only ever see ciphertext.
 */
import { schnorr } from "@noble/curves/secp256k1.js";
import { hush } from "../core/wasm";
import { bytes, toHex } from "../lib/bytes";
import { signEvent } from "../signal/nostr";

export { DEFAULT_BLOSSOM as DEFAULT_SERVERS } from "../signal/defaults";
/** How many servers keep a copy of each blob. */
export const COPIES = 2;
const AUTH_KIND = 24_242;
const TRIES = 3;

/** Plain HTTP only for the local machine (tests and dev), as in the core. */
export function serverUrl(host: string): string {
  const local = /^(localhost|127\.0\.0\.1)(:\d+)?$/.test(host);
  return `${local ? "http" : "https"}://${host}`;
}

/** A host as a locator names it: `name[:port]`, lowercase, no scheme or path. */
export function cleanHost(input: string): string | null {
  const h = input
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .replace(/\/+$/, "");
  return /^[a-z0-9][a-z0-9.:-]{0,252}$/.test(h) && !h.endsWith(".p") ? h : null;
}

/** A fresh throwaway secp256k1 secret (kept in the manifest, so whoever opens it can delete). */
export function newDeleteKey(): Uint8Array {
  return schnorr.utils.randomSecretKey();
}

function auth(sk: Uint8Array, verb: string, sha: string): string {
  const now = Math.floor(Date.now() / 1000);
  const ev = signEvent(
    sk,
    verb,
    [
      ["t", verb],
      ["x", sha],
      ["expiration", String(now + 3600)],
    ],
    AUTH_KIND,
  );
  return `Nostr ${btoa(JSON.stringify(ev))}`;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function putOnce(host: string, data: Uint8Array, sha: string, sk: Uint8Array): Promise<void> {
  const res = await fetch(`${serverUrl(host)}/upload`, {
    method: "PUT",
    body: bytes(data),
    headers: {
      Authorization: auth(sk, "upload", sha),
      "Content-Type": "application/octet-stream",
      "X-SHA-256": sha,
    },
    credentials: "omit",
    referrerPolicy: "no-referrer",
  });
  if (!res.ok) {
    const why = res.headers.get("x-reason");
    throw new Error(`${host}: ${res.status}${why ? ` ${why.slice(0, 120)}` : ""}`);
  }
  const desc = (await res.json().catch(() => null)) as { sha256?: string } | null;
  if (desc?.sha256 && desc.sha256 !== sha) throw new Error(`${host} stored something else`);
}

/** Uploads a blob to one server, retrying. */
export async function put(host: string, data: Uint8Array, sk: Uint8Array): Promise<void> {
  const sha = toHex(hush().sha256(data));
  let last: unknown;
  for (let i = 0; i < TRIES; i++) {
    try {
      return await putOnce(host, data, sha, sk);
    } catch (e) {
      last = e;
      await sleep(500 << i);
    }
  }
  throw last;
}

/** Uploads to `want` of `hosts`, in order; returns those that took it. */
export async function putAny(
  hosts: string[],
  want: number,
  data: Uint8Array,
  sk: Uint8Array,
): Promise<string[]> {
  const ok: string[] = [];
  const errors: string[] = [];
  for (const h of hosts) {
    if (ok.length >= want) break;
    try {
      await put(h, data, sk);
      ok.push(h);
    } catch (e) {
      errors.push(e instanceof Error ? e.message : String(e));
    }
  }
  if (ok.length === 0) throw new Error(errors.join("; ") || "no server");
  return ok;
}

/** Fetches a blob from the first host that has it, checking its SHA-256. */
export async function get(
  hosts: string[],
  sha: Uint8Array,
  max: number,
  signal?: AbortSignal,
): Promise<Uint8Array> {
  const hex = toHex(sha);
  const errors: string[] = [];
  for (let i = 0; i < TRIES; i++) {
    for (const h of hosts) {
      try {
        const res = await fetch(`${serverUrl(h)}/${hex}`, {
          credentials: "omit",
          referrerPolicy: "no-referrer",
          ...(signal ? { signal } : {}),
        });
        if (!res.ok) throw new Error(`${h}: ${res.status}`);
        const len = Number(res.headers.get("content-length") ?? 0);
        if (len > max) throw new Error(`${h}: too large`);
        const body = new Uint8Array(await res.arrayBuffer());
        if (body.length > max) throw new Error(`${h}: too large`);
        if (toHex(hush().sha256(body)) !== hex) throw new Error(`${h} returned other bytes`);
        return body;
      } catch (e) {
        if (signal?.aborted) throw e;
        errors.push(e instanceof Error ? e.message : String(e));
      }
    }
    await sleep(500 << i);
  }
  throw new Error([...new Set(errors)].join("; "));
}

/** Asks every host to delete a blob; resolves to how many confirmed. */
export async function del(hosts: string[], sha: Uint8Array, sk: Uint8Array): Promise<number> {
  const hex = toHex(sha);
  const done = await Promise.all(
    hosts.map((h) =>
      fetch(`${serverUrl(h)}/${hex}`, {
        method: "DELETE",
        headers: { Authorization: auth(sk, "delete", hex) },
        credentials: "omit",
        referrerPolicy: "no-referrer",
      })
        .then((r) => r.ok || r.status === 404)
        .catch(() => false),
    ),
  );
  return done.filter(Boolean).length;
}
