import { expect, test } from "@playwright/test";
import { open, person } from "./helpers";

/** What the sealed store holds for the TURN password, and which hush databases exist. */
function atRest(page: import("@playwright/test").Page, secret: string) {
  return page.evaluate(async (secret) => {
    const record = await new Promise<{ v?: number; ciphertext?: ArrayBuffer } | null>((resolve) => {
      const open = indexedDB.open("hush-sealed");
      open.onupgradeneeded = () => open.transaction?.abort();
      open.onerror = () => resolve(null);
      open.onsuccess = () => {
        const get = open.result.transaction("kv").objectStore("kv").get("turn-credential");
        get.onsuccess = () => resolve(get.result ?? null);
        get.onerror = () => resolve(null);
      };
    });
    const bytes = record?.ciphertext ? new TextDecoder("latin1").decode(record.ciphertext) : "";
    return {
      v: record?.v ?? null,
      clear: bytes.includes(secret),
      databases: (await indexedDB.databases())
        .map((d) => d.name ?? "")
        .filter((n) => n.includes("secret") || n.includes("sealed"))
        .sort(),
    };
  }, secret);
}

test("the TURN password is never kept in clear, and survives a reload", async ({ browser }) => {
  const alice = await person(browser, "Alice");
  await open(alice);
  const page = alice.page;
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByPlaceholder("turn:turn.example.org:3478").fill("turn:relay.example:3478");
  await page.getByPlaceholder("username").fill("alice");
  await page.getByPlaceholder("credential").fill("hunter2-turn");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("button", { name: "Saved" })).toBeVisible();
  await expect(page.getByTestId("settings-unsaved")).toHaveCount(0);

  const stored = await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }));
  expect(stored).toContain("relay.example");
  expect(stored).not.toContain("hunter2-turn");
  expect(await atRest(page, "hunter2-turn")).toEqual({
    v: 2,
    clear: false,
    databases: ["hush-sealed"],
  });

  await page.reload();
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.getByPlaceholder("credential")).toHaveValue("hunter2-turn");
  expect(alice.errors).toEqual([]);
});

test("a full storage is reported instead of claiming the change was saved", async ({ browser }) => {
  const alice = await person(browser, "Alice");
  await open(alice);
  const page = alice.page;
  await page.getByRole("button", { name: "Settings" }).click();
  await page.evaluate(() => {
    Storage.prototype.setItem = () => {
      throw new DOMException("full", "QuotaExceededError");
    };
  });
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByTestId("settings-unsaved")).toBeVisible();
  await expect(page.getByRole("button", { name: "Saved" })).toHaveCount(0);
});

test("a TURN password sealed by an earlier version moves to the sealed store", async ({
  browser,
}) => {
  const turn = { url: "turn:relay.example:3478", username: "alice", credential: "" };
  const alice = await person(browser, "Alice", { turn });
  await open(alice);
  const page = alice.page;
  // What an earlier version's own code left: { iv, data } under "key", the name as additional data.
  await page.evaluate(async (secret) => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const open = indexedDB.open("hush-secrets", 1);
      open.onupgradeneeded = () => open.result.createObjectStore("kv");
      open.onsuccess = () => resolve(open.result);
      open.onerror = () => reject(open.error);
    });
    const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, [
      "encrypt",
      "decrypt",
    ]);
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const name = new TextEncoder().encode("turn-credential");
    const data = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv, additionalData: name },
      key,
      new TextEncoder().encode(secret),
    );
    await new Promise((resolve, reject) => {
      const tx = db.transaction("kv", "readwrite");
      tx.objectStore("kv").put(key, "key");
      tx.objectStore("kv").put({ iv, data }, "turn-credential");
      tx.oncomplete = resolve;
      tx.onerror = () => reject(tx.error);
    });
    db.close();
  }, "old-turn-pass");
  await page.reload();
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.getByPlaceholder("credential")).toHaveValue("old-turn-pass");
  await expect
    .poll(() => atRest(page, "old-turn-pass"))
    .toEqual({ v: 2, clear: false, databases: ["hush-sealed"] });
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage }))).not.toContain(
    "old-turn-pass",
  );
  expect(alice.errors).toEqual([]);
});
