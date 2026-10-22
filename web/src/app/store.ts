/**
 * The vault: the only thing hush ever writes to disk, and only if the user
 * creates one. IndexedDB holds the vault header (the data key wrapped by
 * Argon2id(passphrase)) and records sealed by the core, under opaque HMAC
 * keys. Without the passphrase it is noise, even to other pages of the same
 * origin (github.io serves several sites from one origin).
 *
 * Backups are the same data as one JSON snapshot, sealed by the core's backup
 * format; the CLI reads and writes the same files.
 */
import { signal } from "@preact/signals";
import { hush, type Identity, normalizePassphrase, type Vault } from "../core/wasm";
import { fromB64u, fromUtf8, toB64u, toHex, utf8 } from "../lib/bytes";

export interface Contact {
  /** `hush1…` */
  id: string;
  /** Hex of the identifier hash (record id). */
  key: string;
  name: string;
  /** Latest signed bundle (base64url). */
  bundle: string;
  /** Shared contact secret from the session where cards were exchanged (base64url). */
  secret: string;
  added: number;
  history: boolean;
}

export interface HistoryEntry {
  id: string;
  mine: boolean;
  ts: number;
  text: string;
  expiresAt?: number;
}

/** What a backup file contains (also written by `hush backup export`). */
export interface Snapshot {
  v: 1;
  identity: { seed: string; name: string } | null;
  contacts: Contact[];
  history: Record<string, HistoryEntry[]>;
}

export type VaultState = "loading" | "none" | "locked" | "unlocked";

export const vaultState = signal<VaultState>("loading");
export const identity = signal<Identity | null>(null);
export const contacts = signal<Contact[]>([]);

const DB = "hush";
const STORE = "kv";
const HEADER = "vault-header";
const MAX_HISTORY = 2000;

let vault: Vault | null = null;

function db(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function tx<T>(
  mode: IDBTransactionMode,
  fn: (s: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  const d = await db();
  return new Promise((resolve, reject) => {
    const req = fn(d.transaction(STORE, mode).objectStore(STORE));
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  }).finally(() => d.close()) as Promise<T>;
}

const getRaw = (k: string) =>
  tx<Uint8Array | undefined>("readonly", (s) => s.get(k) as IDBRequest<Uint8Array | undefined>);
const putRaw = (k: string, v: Uint8Array) => tx("readwrite", (s) => s.put(v, k));
const delRaw = (k: string) => tx("readwrite", (s) => s.delete(k));

function need(): Vault {
  if (!vault) throw new Error("vault locked");
  return vault;
}

async function readRecord<T>(kind: string, id: string): Promise<T | null> {
  const v = need();
  const blob = await getRaw(`r:${v.index(kind, id)}`);
  if (!blob) return null;
  return JSON.parse(fromUtf8(v.open(kind, id, blob))) as T;
}

async function writeRecord(kind: string, id: string, value: unknown): Promise<void> {
  const v = need();
  await putRaw(`r:${v.index(kind, id)}`, v.seal(kind, id, utf8(JSON.stringify(value))));
}

async function deleteRecord(kind: string, id: string): Promise<void> {
  await delRaw(`r:${need().index(kind, id)}`);
}

/** Looks for an existing vault. */
export async function initStore(): Promise<void> {
  try {
    vaultState.value = (await getRaw(HEADER)) ? "locked" : "none";
  } catch {
    vaultState.value = "none"; // IndexedDB unavailable (private mode): nothing can be kept
  }
}

async function loadAll(): Promise<void> {
  const me = await readRecord<{ seed: string; name: string }>("identity", "me");
  identity.value = me ? hush().Identity.fromSeed(fromB64u(me.seed)) : null;
  const keys = (await readRecord<string[]>("index", "contacts")) ?? [];
  const list: Contact[] = [];
  for (const k of keys) {
    const c = await readRecord<Contact>("contact", k);
    if (c) list.push(c);
  }
  contacts.value = list;
  void navigator.storage?.persist?.().catch(() => false);
}

export async function createVault(passphrase: string): Promise<void> {
  const res = hush().Vault.create(normalizePassphrase(passphrase)) as {
    vault: Vault;
    header: Uint8Array;
  };
  await putRaw(HEADER, res.header);
  vault = res.vault;
  await writeRecord("index", "contacts", []);
  vaultState.value = "unlocked";
  await loadAll();
}

/** Throws `Decrypt` for a wrong passphrase. */
export async function unlockVault(passphrase: string): Promise<void> {
  const header = await getRaw(HEADER);
  if (!header) throw new Error("no vault");
  vault = hush().Vault.unlock(normalizePassphrase(passphrase), header);
  vaultState.value = "unlocked";
  await loadAll();
}

export function lockVault(): void {
  vault?.free();
  vault = null;
  identity.value?.free();
  identity.value = null;
  contacts.value = [];
  vaultState.value = "locked";
}

export async function eraseVault(): Promise<void> {
  lockVault();
  await new Promise<void>((resolve) => {
    const req = indexedDB.deleteDatabase(DB);
    req.onsuccess = req.onerror = req.onblocked = () => resolve();
  });
  vaultState.value = "none";
}

export async function changePassphrase(passphrase: string): Promise<void> {
  await putRaw(HEADER, need().rewrap(normalizePassphrase(passphrase)));
}

// ------------------------------------------------------------------------------------------
// Identity and contacts

export async function ensureIdentity(name: string): Promise<Identity> {
  if (identity.value) return identity.value;
  const id = hush().Identity.generate();
  await writeRecord("identity", "me", { seed: toB64u(id.seed()), name });
  identity.value = id;
  return id;
}

export async function saveContact(c: Contact): Promise<void> {
  await writeRecord("contact", c.key, c);
  const rest = contacts.value.filter((x) => x.key !== c.key);
  contacts.value = [...rest, c];
  await writeRecord(
    "index",
    "contacts",
    contacts.value.map((x) => x.key),
  );
}

export async function removeContact(key: string): Promise<void> {
  await deleteRecord("contact", key);
  await deleteRecord("history", key);
  contacts.value = contacts.value.filter((x) => x.key !== key);
  await writeRecord(
    "index",
    "contacts",
    contacts.value.map((x) => x.key),
  );
}

export async function loadHistory(key: string): Promise<HistoryEntry[]> {
  const now = Date.now();
  return ((await readRecord<HistoryEntry[]>("history", key)) ?? []).filter(
    (e) => !e.expiresAt || e.expiresAt > now,
  );
}

export async function appendHistory(key: string, entry: HistoryEntry): Promise<void> {
  const list = await loadHistory(key);
  list.push(entry);
  await writeRecord("history", key, list.slice(-MAX_HISTORY));
}

export async function removeHistory(key: string, id: string): Promise<void> {
  const list = await loadHistory(key);
  const kept = list.filter((e) => e.id !== id);
  if (kept.length !== list.length) await writeRecord("history", key, kept);
}

export async function clearHistory(key: string): Promise<void> {
  await deleteRecord("history", key);
}

// ------------------------------------------------------------------------------------------
// Backups

export async function snapshot(): Promise<Snapshot> {
  const me = await readRecord<{ seed: string; name: string }>("identity", "me");
  const history: Record<string, HistoryEntry[]> = {};
  for (const c of contacts.value) {
    const h = await loadHistory(c.key);
    if (h.length) history[c.key] = h;
  }
  return { v: 1, identity: me, contacts: contacts.value, history };
}

export async function exportBackup(passphrase: string): Promise<Blob> {
  const snap = await snapshot();
  const file = hush().backupExport(normalizePassphrase(passphrase), utf8(JSON.stringify(snap)));
  return new Blob([file as Uint8Array<ArrayBuffer>], { type: "application/octet-stream" });
}

function validSnapshot(v: unknown): v is Snapshot {
  const s = v as Snapshot;
  return !!s && s.v === 1 && Array.isArray(s.contacts) && typeof s.history === "object";
}

/** Merges a backup into the (unlocked) vault. Throws `Decrypt` for a wrong passphrase. */
export async function importBackup(file: Uint8Array, passphrase: string): Promise<number> {
  const plain = hush().backupImport(normalizePassphrase(passphrase), file);
  const snap: unknown = JSON.parse(fromUtf8(plain));
  if (!validSnapshot(snap)) throw new Error("Malformed");
  if (snap.identity && !identity.value) {
    await writeRecord("identity", "me", snap.identity);
    identity.value = hush().Identity.fromSeed(fromB64u(snap.identity.seed));
  }
  let n = 0;
  for (const c of snap.contacts) {
    try {
      // Re-verify every bundle: a backup is untrusted input like any other.
      const b = hush().parseStoredBundle(fromB64u(c.bundle)) as { id: string; idHash: Uint8Array };
      if (b.id !== c.id || toHex(b.idHash) !== c.key || fromB64u(c.secret).length !== 32) continue;
    } catch {
      continue;
    }
    await saveContact(c);
    const h = snap.history[c.key];
    if (Array.isArray(h)) await writeRecord("history", c.key, h.slice(-MAX_HISTORY));
    n++;
  }
  return n;
}

export const isUnlocked = () => vaultState.value === "unlocked" && vault !== null;
