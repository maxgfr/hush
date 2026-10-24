import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { BASE, close, open, person, unzip } from "./helpers";

// Chromium hides host IPs behind mDNS names, which CI runners cannot resolve.
test.use({ launchOptions: { args: ["--disable-features=WebRtcHideLocalIpsWithMdns"] } });

const sha = (b: Buffer | Uint8Array) => createHash("sha256").update(b).digest("hex");

test("a folder dropped between two browsers goes peer to peer while both are there", async ({
  browser,
}) => {
  const dir = mkdtempSync(join(tmpdir(), "hush-drop-"));
  mkdirSync(join(dir, "album", "sub"), { recursive: true });
  const big = Buffer.from(Array.from({ length: 5 * 1024 * 1024 + 1 }, (_, i) => (i * 29) & 255));
  writeFileSync(join(dir, "album", "big.bin"), big);
  writeFileSync(join(dir, "album", "sub", "note.txt"), "bonjour");

  const alice = await person(browser, "Alice");
  await open(alice);
  await alice.page.getByTestId("home-send-files").click();
  await alice.page.getByTestId("drop-folder").setInputFiles(join(dir, "album"));
  await expect(alice.page.getByText("album/sub/note.txt")).toBeVisible();
  await alice.page.getByTestId("drop-send").click();
  await expect(alice.page.getByTestId("drop-ready")).toBeVisible({ timeout: 30_000 });
  const link = (await alice.page.getByTestId("drop-link").textContent()) ?? "";
  expect(link).toMatch(/#2\.d\./);

  const bob = await person(browser, "Bob");
  await bob.page.goto(link.replace("#", "?nosw#"));
  await expect(bob.page.getByText("The sender is online: direct transfer")).toBeVisible({
    timeout: 30_000,
  });
  const download = bob.page.waitForEvent("download");
  await bob.page.getByTestId("drop-download").click();
  const zip = await download;
  expect(zip.suggestedFilename()).toBe("album.zip");
  const entries = unzip(readFileSync(await zip.path()));
  expect(entries.map((e) => e.name).sort()).toEqual(["album/big.bin", "album/sub/note.txt"]);
  expect(sha(entries.find((e) => e.name === "album/big.bin")?.data as Buffer)).toBe(sha(big));
  await expect(bob.page.getByTestId("drop-saved")).toBeVisible();
  await expect(alice.page.getByText("Delivered directly")).toBeVisible();
  // One-time: delivered peer to peer, the copy is gone from the servers too.
  await expect(alice.page.getByText("Deleted from the servers")).toBeVisible();
  await close(alice, bob);
});

test("the sealed copy waits behind a passphrase, and a one-time link works once", async ({
  browser,
}) => {
  const data = Buffer.from(Array.from({ length: 700_000 }, (_, i) => (i * 3) & 255));
  const alice = await person(browser, "Alice");
  await open(alice);
  await alice.page.getByTestId("home-send-files").click();
  await alice.page
    .getByTestId("drop-files")
    .setInputFiles({ name: "contrat.pdf", mimeType: "application/pdf", buffer: data });
  await alice.page.getByLabel("Passphrase (optional)").fill("deux mots");
  await alice.page.getByTestId("drop-send").click();
  await expect(alice.page.getByTestId("drop-ready")).toBeVisible({ timeout: 30_000 });
  const link = ((await alice.page.getByTestId("drop-link").textContent()) ?? "").replace(
    "#",
    "?nosw#",
  );
  expect(link).toMatch(/\.p$/);
  // The sender leaves: only the sealed copy remains.
  await close(alice);

  const bob = await person(browser, "Bob");
  await bob.page.goto(link);
  await bob.page.locator('input[type="password"]').fill("trois mots");
  await bob.page.getByRole("button", { name: "Join" }).click();
  await expect(bob.page.getByText("Wrong passphrase.")).toBeVisible({ timeout: 30_000 });

  await bob.page.goto(`${BASE}/?nosw`);
  await bob.page.goto(link);
  await bob.page.locator('input[type="password"]').fill("deux mots");
  await bob.page.getByRole("button", { name: "Join" }).click();
  await expect(bob.page.getByText("contrat.pdf")).toBeVisible({ timeout: 30_000 });
  await expect(bob.page.getByText("From the sealed copy")).toBeVisible();
  const download = bob.page.waitForEvent("download");
  await bob.page.getByTestId("drop-download").click();
  const file = await download;
  expect(file.suggestedFilename()).toBe("contrat.pdf");
  expect(sha(readFileSync(await file.path()))).toBe(sha(data));
  await expect(bob.page.getByText("One-time link: the copy was deleted")).toBeVisible();

  await bob.page.goto(`${BASE}/?nosw`);
  await bob.page.goto(link);
  await bob.page.locator('input[type="password"]').fill("deux mots");
  await bob.page.getByRole("button", { name: "Join" }).click();
  await expect(bob.page.getByText("This drop is no longer on its servers")).toBeVisible({
    timeout: 30_000,
  });
  await close(bob);
});

test("in a conversation without a direct path, a file comes through a sealed copy", async ({
  browser,
}) => {
  test.setTimeout(180_000);
  // Only TURN, and no TURN server: the peers never get a direct path, only the relays.
  const alice = await person(browser, "Alice", { relayOnly: true });
  const bob = await person(browser, "Bob", { relayOnly: true });
  await open(alice);
  await alice.page.getByRole("button", { name: "New conversation" }).click();
  await alice.page.getByRole("button", { name: "Create the link" }).click();
  const link = (await alice.page.getByTestId("share-link").textContent()) ?? "";
  await bob.page.goto(link.replace("#", "?nosw#"));
  await expect(alice.page.getByTestId("sas").first()).toBeVisible({ timeout: 30_000 });
  const data = Buffer.from(Array.from({ length: 200_000 }, (_, i) => (i * 11) & 255));
  await alice.page
    .locator('input[type="file"]')
    .setInputFiles({ name: "relayed.bin", mimeType: "application/octet-stream", buffer: data });
  await expect(bob.page.getByText("relayed.bin")).toBeVisible({ timeout: 30_000 });
  // Past the wait for a direct path, the download asks for a sealed copy.
  await expect(bob.page.locator(".peer-badge.relay").first()).toBeVisible({ timeout: 60_000 });
  await bob.page.getByRole("button", { name: "Download" }).click();
  await expect(bob.page.getByText("through a sealed copy")).toBeVisible();
  const download = bob.page.waitForEvent("download");
  await bob.page.locator("a[download='relayed.bin']").click({ timeout: 60_000 });
  expect(sha(readFileSync(await (await download).path()))).toBe(sha(data));
  await expect(alice.page.getByText("delivered ✓")).toBeVisible();
  await close(alice, bob);
});
