/**
 * Minimal Blossom server (BUD-01/02) for tests and local dev, on the app's origin: `PUT /upload`,
 * `GET|HEAD|DELETE /<sha256>`. Like a real server it addresses blobs by their SHA-256 and checks
 * that uploads and deletions carry a signed kind-24242 authorization for that exact blob.
 */
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { type RelayEvent, verifyEvent } from "./mock-relay";

export interface MockBlossom {
  /** Handles a Blossom request, or returns null for any other path. */
  handle(req: Request): Promise<Response | null>;
  readonly blobs: Map<string, Uint8Array>;
  /** Refuse every request (a server that went away). */
  down: boolean;
}

const BLOB = /^\/([0-9a-f]{64})$/;

function authorized(req: Request, verb: string, x: string): boolean {
  const header = req.headers.get("authorization") ?? "";
  if (!header.startsWith("Nostr ")) return false;
  try {
    const ev = JSON.parse(atob(header.slice(6))) as RelayEvent;
    return (
      verifyEvent(ev) &&
      ev.kind === 24242 &&
      ev.content === verb &&
      ev.tags.some((t) => t[0] === "t" && t[1] === verb) &&
      ev.tags.some((t) => t[0] === "x" && t[1] === x)
    );
  } catch {
    return false;
  }
}

export function createMockBlossom(): MockBlossom {
  const blobs = new Map<string, Uint8Array>();
  const self: MockBlossom = {
    blobs,
    down: false,
    async handle(req) {
      const path = new URL(req.url).pathname;
      const blob = BLOB.exec(path)?.[1];
      if (path !== "/upload" && !blob) return null;
      if (self.down) return new Response("down", { status: 503 });
      if (path === "/upload" && req.method === "PUT") {
        const body = new Uint8Array(await req.arrayBuffer());
        const x = bytesToHex(sha256(body));
        if (req.headers.get("x-sha-256") !== x || !authorized(req, "upload", x)) {
          return new Response("unauthorized", {
            status: 401,
            headers: { "x-reason": "bad authorization" },
          });
        }
        blobs.set(x, body);
        return Response.json({ sha256: x, size: body.length, url: `/${x}` }, { status: 201 });
      }
      if (!blob) return new Response("bad request", { status: 400 });
      if (req.method === "DELETE") {
        if (!authorized(req, "delete", blob)) return new Response("unauthorized", { status: 401 });
        blobs.delete(blob);
        return new Response(null, { status: 204 });
      }
      const data = blobs.get(blob);
      if (!data) return new Response("not found", { status: 404 });
      return new Response(req.method === "HEAD" ? null : data, {
        headers: { "content-type": "application/octet-stream", "content-length": String(data.length) },
      });
    },
  };
  return self;
}
