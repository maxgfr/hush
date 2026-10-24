/**
 * Offline-first service worker. Precaches the hashed build (the list, the
 * version and each file's SHA-256 are injected at build time) and serves it
 * cache-first. A new version installs in the background and waits: the page
 * shows "Update available" and only then asks it to take over, so a call is
 * never interrupted by an update.
 *
 * Integrity: Cache Storage belongs to the whole origin, and every
 * `maxgfr.github.io/*` site shares it, so any of them could rewrite hush's
 * cached files. This worker's own script lives in its registration, out of
 * their reach, and it only serves a cached file whose SHA-256 matches the hash
 * built into it. Anything else is never served: the cache is dropped and the
 * file comes from the network. Only the body is taken from the cache: status
 * and headers are set here, since another page could have stored a redirect
 * (which would carry the #link fragment away) around genuine bytes.
 *
 * Downloads: a page of hush can hand this worker a stream (a MessagePort) for
 * a large download; `./__drop/<id>` then answers with it as an attachment, so
 * the browser writes it to disk as it arrives instead of the page holding it
 * all in memory. Each id is random, served once, and only hush's own pages may
 * register one.
 */

// A module: these interfaces stay local rather than merge with the DOM's.
export {};

interface SwScope {
  addEventListener(type: "install", fn: (e: ExtendableEvent) => void): void;
  addEventListener(type: "activate", fn: (e: ExtendableEvent) => void): void;
  addEventListener(type: "fetch", fn: (e: FetchEvent) => void): void;
  addEventListener(type: "message", fn: (e: MessageEvent) => void): void;
  skipWaiting(): Promise<void>;
  clients: { claim(): Promise<void> };
  location: Location;
  registration: { scope: string };
}

interface ExtendableEvent extends Event {
  waitUntil(p: Promise<unknown>): void;
}

interface FetchEvent extends ExtendableEvent {
  request: Request;
  respondWith(r: Promise<Response> | Response): void;
}

const sw = self as unknown as SwScope;
const CACHE = `hush-${__SW_VERSION__}`;
const PRECACHE = __PRECACHE__;
/** Base64 SHA-256 of each precached file, by absolute URL. */
const INTEGRITY = new Map(
  Object.entries(__INTEGRITY__).map(([path, hash]) => [new URL(path, sw.location.href).href, hash]),
);
/** The type of each file the build emits, by extension; nothing else comes from the cache. */
const TYPES: Record<string, string> = {
  html: "text/html; charset=utf-8",
  js: "text/javascript; charset=utf-8",
  css: "text/css; charset=utf-8",
  wasm: "application/wasm",
  webmanifest: "application/manifest+json",
  json: "application/json",
  svg: "image/svg+xml",
  png: "image/png",
};
const typeOf = (url: string): string => {
  if (url.endsWith("/")) return "text/html; charset=utf-8";
  return TYPES[new URL(url).pathname.split(".").pop() ?? ""] ?? "application/octet-stream";
};
/** What an older version left behind. */
const stale = (key: string) => key.startsWith("hush-") && key !== CACHE;

async function sha256(body: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", body));
  return btoa(String.fromCharCode(...digest));
}

/** The cached copy of `url`, only if it is exactly the file this version was built with. */
async function verified(cache: Cache, url: string): Promise<Response | null> {
  const expected = INTEGRITY.get(url);
  if (!expected) return null;
  const hit = await cache.match(url, { ignoreSearch: true });
  if (!hit) return null;
  const body = await hit.arrayBuffer();
  if ((await sha256(body)) !== expected) {
    // Someone else wrote into our cache: forget all of it and go to the network.
    await caches.delete(CACHE);
    return null;
  }
  return new Response(body, { status: 200, headers: { "Content-Type": typeOf(url) } });
}

sw.addEventListener("install", (e) => {
  e.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      for (const url of PRECACHE) {
        // Past the HTTP cache: a shell cached minutes earlier names scripts the new deploy removed.
        const res = await fetch(new Request(url, { cache: "reload" }));
        const body = await res.clone().arrayBuffer();
        const expected = INTEGRITY.get(new URL(url, sw.location.href).href);
        // A CDN still serving the previous deploy: fail the install, the browser retries later.
        if (!res.ok || (await sha256(body)) !== expected) throw new Error(`integrity: ${url}`);
        await cache.put(url, res);
      }
    })(),
  );
});

sw.addEventListener("activate", (e) => {
  e.waitUntil(
    (async () => {
      for (const key of await caches.keys()) if (stale(key)) await caches.delete(key);
      await sw.clients.claim();
    })(),
  );
});

/** Downloads waiting for the browser to fetch them, by id. */
const streams = new Map<string, { port: MessagePort; name: string }>();

sw.addEventListener("message", (e) => {
  // Only hush's own pages may switch versions: any page of the origin can post to us.
  const from = (e.source as { url?: string } | null)?.url ?? "";
  if (!from.startsWith(sw.registration.scope)) return;
  const data = e.data as { type?: string; id?: unknown; name?: unknown } | null;
  if (data?.type === "skip-waiting") void sw.skipWaiting();
  const port = e.ports[0];
  if (
    data?.type === "drop-stream" &&
    port &&
    typeof data.id === "string" &&
    /^[0-9a-f]{32}$/.test(data.id)
  ) {
    const name = typeof data.name === "string" ? data.name.slice(0, 255) : "file";
    streams.set(data.id, { port, name });
    // Forgotten if the page never asks for it.
    setTimeout(() => streams.delete(data.id as string), 60_000);
    port.postMessage({ ready: true });
  }
});

/** The stream registered as `id`, as an attachment, pulled from the page chunk by chunk. */
function streamed(id: string): Response {
  const s = streams.get(id);
  if (!s) return new Response("gone", { status: 404 });
  streams.delete(id);
  const { port, name } = s;
  const body = new ReadableStream<Uint8Array>({
    pull(ctrl) {
      return new Promise<void>((resolve) => {
        port.onmessage = (m) => {
          const d = m.data as { chunk?: ArrayBuffer; done?: boolean; error?: boolean };
          if (d.chunk) ctrl.enqueue(new Uint8Array(d.chunk));
          else if (d.done) ctrl.close();
          else ctrl.error(new Error("aborted"));
          resolve();
        };
        port.postMessage({ pull: true });
      });
    },
    cancel() {
      port.postMessage({ cancel: true });
    },
  });
  const safe = name.replace(/["\\\r\n]/g, "_");
  return new Response(body, {
    status: 200,
    headers: {
      "Content-Type": "application/octet-stream",
      "Content-Disposition": `attachment; filename="${safe.replace(/[^\x20-\x7e]/g, "_")}"; filename*=UTF-8''${encodeURIComponent(name)}`,
      "X-Content-Type-Options": "nosniff",
    },
  });
}

sw.addEventListener("fetch", (e) => {
  const req = e.request;
  const url = new URL(req.url);
  if (req.method !== "GET" || url.origin !== sw.location.origin) return;
  const drop = /\/__drop\/([0-9a-f]{32})$/.exec(url.pathname);
  if (drop?.[1] && url.href.startsWith(sw.registration.scope)) {
    e.respondWith(streamed(drop[1]));
    return;
  }
  e.respondWith(
    (async () => {
      // Opening a cache creates it: once a tampered one is dropped, stay on the network.
      if (!(await caches.has(CACHE))) return fetch(req);
      const cache = await caches.open(CACHE);
      // Navigations (any #link fragment, any query) get the app shell.
      if (req.mode === "navigate") {
        const shell = await verified(cache, new URL("./", sw.location.href).href);
        if (shell) return shell;
      }
      url.search = "";
      return (await verified(cache, url.href)) ?? fetch(req);
    })(),
  );
});
