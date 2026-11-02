import { beforeEach, describe, expect, mock, test } from "bun:test";

// The sealed store (IndexedDB + WebCrypto) as a map; `secretsWork` switches it off.
const sealed = new Map<string, string>();
let secretsWork = true;
mock.module("./secrets", () => ({
  writeSecret: async (name: string, value: string) => {
    if (!value) {
      sealed.delete(name);
      return true;
    }
    if (!secretsWork) return false;
    sealed.set(name, value);
    return true;
  },
  readSecret: async (name: string) => (secretsWork ? (sealed.get(name) ?? null) : null),
}));

const KEY = "hush.settings.v1";
const store = new Map<string, string>();
let full = false;
globalThis.localStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => {
    if (full) throw new DOMException("quota", "QuotaExceededError");
    store.set(k, v);
  },
  removeItem: (k: string) => void store.delete(k),
  clear: () => store.clear(),
  key: () => null,
  length: 0,
} as Storage;

const turn = { url: "turn:relay.example:3478", username: "me", credential: "s3cret" };
// Saved in clear by an earlier version, before the module loads.
store.set(KEY, JSON.stringify({ name: "Ana", turn }));
const mod = await import("./settings");

beforeEach(() => {
  full = false;
  secretsWork = true;
});

describe("settings on a shared origin", () => {
  test("a TURN credential saved in clear moves to the sealed store", async () => {
    expect(mod.settings.value.turn?.credential).toBe("s3cret");
    await mod.loadTurnCredential();
    expect(sealed.get("turn-credential")).toBe("s3cret");
    expect(store.get(KEY)).not.toContain("s3cret");
    expect(JSON.parse(store.get(KEY) ?? "{}").name).toBe("Ana");
    expect(mod.iceServers().at(-1)).toEqual({
      urls: turn.url,
      username: "me",
      credential: "s3cret",
    });
  });

  test("the credential comes back from the sealed store after a reload", async () => {
    mod.settings.value = { ...mod.settings.value, turn: { ...turn, credential: "" } };
    await mod.loadTurnCredential();
    expect(mod.settings.value.turn?.credential).toBe("s3cret");
  });

  test("saving never writes the credential to localStorage", async () => {
    expect(await mod.saveTurn({ ...turn, credential: "other" })).toBe(true);
    expect(store.get(KEY)).not.toContain("other");
    expect(sealed.get("turn-credential")).toBe("other");
  });

  test("a full storage is reported, and the change lasts for the page", async () => {
    full = true;
    expect(mod.saveSettings({ name: "Bea" })).toBe(false);
    expect(mod.settings.value.name).toBe("Bea");
    expect(JSON.parse(store.get(KEY) ?? "{}").name).toBe("Ana");
    full = false;
    secretsWork = false;
    expect(await mod.saveTurn(turn)).toBe(false);
  });
});
