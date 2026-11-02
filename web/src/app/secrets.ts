/**
 * Small secrets kept outside the vault (today: the TURN server's credential),
 * in llm-connect's sealedStore: AES-GCM under a non-extractable key, both in
 * IndexedDB (`hush-sealed`, apart from the vault's database so erasing the vault
 * keeps them), a fresh IV on every write and the name bound to the record. A
 * dump of the origin's storage only yields opaque bytes; a script of another
 * `maxgfr.github.io` site could still ask the browser to decrypt, which is why
 * the UI asks for a login made for this relay only.
 *
 * Every access is guarded: without IndexedDB or WebCrypto, nothing is kept.
 */
import { sealedStore } from "llm-connect";
import { fromUtf8, utf8 } from "../lib/bytes";

const sealed = sealedStore({ dbName: "hush-sealed" });

/**
 * Where earlier versions sealed secrets with their own code (`{ iv, data }`, the
 * name as additional data, under the key "key"). Read once to move a secret over,
 * then deleted: the name is hush's own.
 */
const LEGACY_DB = "hush-secrets";

function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function readLegacy(name: string): Promise<string | null> {
  if (!sealed.available) return null;
  let db: IDBDatabase | undefined;
  try {
    // An upgrade means there was no such database: aborted, so none is created.
    const open = indexedDB.open(LEGACY_DB);
    open.onupgradeneeded = () => open.transaction?.abort();
    db = await request(open);
    if (!db.objectStoreNames.contains("kv")) return null;
    const store = (d: IDBDatabase) => d.transaction("kv", "readonly").objectStore("kv");
    const key = (await request(store(db).get("key"))) as CryptoKey | undefined;
    const old = (await request(store(db).get(name))) as
      | { iv: Uint8Array<ArrayBuffer>; data: ArrayBuffer }
      | undefined;
    if (!key || !old?.iv || !old.data) return null;
    const plain = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: old.iv, additionalData: utf8(name) },
      key,
      old.data,
    );
    return fromUtf8(new Uint8Array(plain));
  } catch {
    return null;
  } finally {
    db?.close();
  }
}

function dropLegacy(): void {
  try {
    indexedDB.deleteDatabase(LEGACY_DB);
  } catch {
    // Nothing to clean up
  }
}

/** Stores `value` under `name` (empty: forgets it). False if it could not be kept. */
export async function writeSecret(name: string, value: string): Promise<boolean> {
  if (!value) {
    // Nothing to keep: a sealed copy left behind (no storage) is never read without a TURN server.
    await sealed.delete(name);
    return true;
  }
  if (!(await sealed.set(name, value))) return false;
  dropLegacy();
  return true;
}

/** The secret stored under `name`, or null (none, unreadable, or no storage). */
export async function readSecret(name: string): Promise<string | null> {
  const value = await sealed.get(name);
  if (value !== null) return value;
  // One sealed by an earlier version: moved over, its old copy dropped only once stored.
  const legacy = await readLegacy(name);
  if (legacy !== null && (await sealed.set(name, legacy))) dropLegacy();
  return legacy;
}
