/**
 * Small secrets kept outside the vault (today: the TURN server's credential).
 * Never in clear text: AES-GCM under a non-extractable key, both in IndexedDB
 * (`hush-secrets`, apart from the vault's database so erasing the vault keeps
 * them). A dump of the origin's storage only yields opaque bytes; a script of
 * another `maxgfr.github.io` site could still ask the browser to decrypt, which
 * is why the UI asks for a login made for this relay only.
 *
 * Every access is guarded: without IndexedDB or WebCrypto, nothing is kept.
 */
import { fromUtf8, randomBytes, utf8 } from "../lib/bytes";

const DB = "hush-secrets";
const STORE = "kv";
const KEY = "key";

interface Sealed {
  iv: Uint8Array<ArrayBuffer>;
  data: ArrayBuffer;
}

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function tx<T>(mode: IDBTransactionMode, fn: (s: IDBObjectStore) => IDBRequest): Promise<T> {
  const d = await open();
  try {
    return await new Promise<T>((resolve, reject) => {
      const t = d.transaction(STORE, mode);
      const req = fn(t.objectStore(STORE));
      t.oncomplete = () => resolve(req.result as T);
      t.onerror = t.onabort = () => reject(t.error);
    });
  } finally {
    d.close();
  }
}

async function cryptoKey(create: boolean): Promise<CryptoKey | undefined> {
  const existing = await tx<CryptoKey | undefined>("readonly", (s) => s.get(KEY));
  if (existing || !create) return existing;
  const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, [
    "encrypt",
    "decrypt",
  ]);
  await tx("readwrite", (s) => s.put(key, KEY));
  return key;
}

/** Stores `value` under `name` (empty: forgets it). False if it could not be kept. */
export async function writeSecret(name: string, value: string): Promise<boolean> {
  if (!value) {
    // Nothing to keep: a sealed copy left behind (no storage) is never read without a TURN server.
    await tx("readwrite", (s) => s.delete(name)).catch(() => {});
    return true;
  }
  try {
    const key = await cryptoKey(true);
    if (!key) return false;
    const iv = randomBytes(12);
    const data = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv, additionalData: utf8(name) },
      key,
      utf8(value),
    );
    await tx("readwrite", (s) => s.put({ iv, data } satisfies Sealed, name));
    return true;
  } catch {
    return false;
  }
}

/** The secret stored under `name`, or null (none, unreadable, or no storage). */
export async function readSecret(name: string): Promise<string | null> {
  try {
    const sealed = await tx<Sealed | undefined>("readonly", (s) => s.get(name));
    const key = sealed && (await cryptoKey(false));
    if (!sealed || !key) return null;
    const plain = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: sealed.iv, additionalData: utf8(name) },
      key,
      sealed.data,
    );
    return fromUtf8(new Uint8Array(plain));
  } catch {
    return null;
  }
}
