import { type ChildProcess, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { close, createLink, HOST, open, person, sas, say, unzip } from "./helpers";

const BIN = resolve(process.cwd(), "../target", process.env.HUSH_CLI_PROFILE ?? "debug", "hush");
const NET = [
  "--relay",
  `ws://${HOST}/relay`,
  "--no-torrent",
  "--no-stun",
  "--server",
  HOST,
  "--base",
  `http://${HOST}/?nosw`,
];

// Chromium hides host IPs behind mDNS names, which CI runners cannot resolve; real networks also
// offer STUN candidates. Here host IPs are exposed instead.
test.use({ launchOptions: { args: ["--disable-features=WebRtcHideLocalIpsWithMdns"] } });
test.skip(!existsSync(BIN), `build the CLI first (cargo build -p hush-cli): ${BIN}`);

interface Proc {
  child: ChildProcess;
  out: string[];
  err: string[];
  exited: Promise<number | null>;
  write(line: string): void;
}

function run(args: string[]): Proc {
  const child = spawn(BIN, args, { stdio: ["pipe", "pipe", "pipe"] });
  const out: string[] = [];
  const err: string[] = [];
  child.stdout?.on("data", (d: Buffer) => out.push(...d.toString().split("\n").filter(Boolean)));
  child.stderr?.on("data", (d: Buffer) => {
    if (process.env.HUSH_E2E_VERBOSE) process.stderr.write(`[cli] ${d}`);
    err.push(...d.toString().split("\n").filter(Boolean));
  });
  const exited = new Promise<number | null>((r) => child.on("exit", (code) => r(code)));
  return { child, out, err, exited, write: (l) => child.stdin?.write(`${l}\n`) };
}

const sha = (b: Buffer) => createHash("sha256").update(b).digest("hex");

test("hush new prints a link the web app opens", async () => {
  const p = run(["new", "--base", "https://maxgfr.github.io/hush/"]);
  expect(await p.exited).toBe(0);
  expect(p.out[0]).toMatch(
    /^https:\/\/maxgfr\.github\.io\/hush\/#2\.s\.[A-Za-z0-9_-]{43}\.[0-9a-z]+$/,
  );
});

test("a browser and the CLI chat: same emojis, messages both ways, ending erases", async ({
  browser,
}) => {
  const alice = await person(browser, "Alice");
  await open(alice);
  const link = await createLink(alice);
  const cli = run(["chat", link, "--name", "Terminal", ...NET]);
  const emojis = await sas(alice);
  await expect
    .poll(() => cli.err.join("\n"), { timeout: 30_000 })
    .toContain("compare these emojis");
  const line = cli.err.find((l) => l.includes("compare these emojis")) ?? "";
  const names = (line.match(/\(([^)]+)\)$/)?.[1] ?? "").split(", ");
  for (const n of names) expect(emojis).toContain(n);
  await expect(alice.page.getByRole("heading", { name: "Terminal" })).toBeVisible();

  cli.write("/ok");
  await alice.page.getByRole("button", { name: "They match" }).click();
  await expect(alice.page.getByText("Verified")).toBeVisible();
  cli.write("hello from the terminal");
  await expect(alice.page.getByText("hello from the terminal")).toBeVisible();
  await say(alice, "hi terminal");
  await expect.poll(() => cli.out.join("\n")).toContain("Alice: hi terminal");

  cli.write("/quit");
  expect(await cli.exited).toBe(0);
  await expect(
    alice.page.getByRole("heading", { name: "The other person ended the conversation" }),
  ).toBeVisible();
  await close(alice);
});

test("the CLI sends a file to a browser in a conversation (SHA-256 checked)", async ({
  browser,
}) => {
  const dir = mkdtempSync(join(tmpdir(), "hush-cli-"));
  const src = join(dir, "from-cli.bin");
  const data = Buffer.from(
    Array.from({ length: 2 * 1024 * 1024 + 17 }, (_, i) => (i * 31 + 7) & 255),
  );
  writeFileSync(src, data);
  const bob = await person(browser, "Bob");
  await open(bob);
  const link = await createLink(bob);
  const cli = run(["send", src, "--link", link, "--name", "Terminal", ...NET]);
  await expect(bob.page.getByText("from-cli.bin")).toBeVisible({ timeout: 30_000 });
  await bob.page.getByRole("button", { name: "Download" }).click();
  const download = bob.page.waitForEvent("download");
  await bob.page.locator("a[download='from-cli.bin']").click({ timeout: 60_000 });
  const path = await (await download).path();
  expect(sha(readFileSync(path))).toBe(sha(data));
  expect(await cli.exited).toBe(0);
  await close(bob);
});

test("a browser sends a file the CLI receives", async ({ browser }) => {
  const dir = mkdtempSync(join(tmpdir(), "hush-cli-"));
  const alice = await person(browser, "Alice");
  await open(alice);
  const link = await createLink(alice);
  const cli = run(["receive", link, "--out", dir, ...NET]);
  await sas(alice);
  const data = Buffer.from(Array.from({ length: 300_000 }, (_, i) => (i * 7) & 255));
  await alice.page
    .locator('input[type="file"]')
    .setInputFiles({ name: "to-cli.bin", mimeType: "application/octet-stream", buffer: data });
  await expect.poll(() => cli.err.join("\n"), { timeout: 60_000 }).toContain("SHA-256 verified");
  expect(sha(readFileSync(join(dir, "to-cli.bin")))).toBe(sha(data));
  await alice.page.getByTestId("end").click();
  expect(await cli.exited).toBe(0);
  await close(alice);
});

test("the CLI drops a folder: the browser gets it from the sealed copy once the CLI is gone", async ({
  browser,
}) => {
  const dir = mkdtempSync(join(tmpdir(), "hush-cli-"));
  mkdirSync(join(dir, "photos", "2026"), { recursive: true });
  const big = Buffer.from(Array.from({ length: 5 * 1024 * 1024 + 3 }, (_, i) => (i * 13) & 255));
  writeFileSync(join(dir, "photos", "2026", "big.jpg"), big);
  writeFileSync(join(dir, "photos", "notes.txt"), "hello");
  const cli = run(["send", join(dir, "photos"), ...NET]);
  expect(await cli.exited).toBe(0);
  expect(cli.err.join("\n")).toContain("the link works without you now");
  const link = cli.out[0] ?? "";
  expect(link).toContain("#2.d.");
  const bob = await person(browser, "Bob");
  await bob.page.goto(link);
  await expect(bob.page.getByText("photos/2026/big.jpg")).toBeVisible({ timeout: 30_000 });
  const download = bob.page.waitForEvent("download");
  await bob.page.getByTestId("drop-download").click();
  const zip = await download;
  expect(zip.suggestedFilename()).toBe("photos.zip");
  const entries = unzip(readFileSync(await zip.path()));
  expect(entries.map((e) => e.name)).toEqual(["photos/2026/big.jpg", "photos/notes.txt"]);
  expect(sha(entries[0]?.data as Buffer)).toBe(sha(big));
  await expect(bob.page.getByTestId("drop-saved")).toBeVisible();
  await close(bob);
});

test("a browser drops files, the CLI receives them (peer to peer, then from the copy)", async ({
  browser,
}) => {
  const alice = await person(browser, "Alice");
  await open(alice);
  await alice.page.getByTestId("home-send-files").click();
  const data = Buffer.from(Array.from({ length: 300_000 }, (_, i) => (i * 7) & 255));
  await alice.page.getByTestId("drop-files").setInputFiles([
    { name: "a.bin", mimeType: "application/octet-stream", buffer: data },
    { name: "b.txt", mimeType: "text/plain", buffer: Buffer.from("b") },
  ]);
  await alice.page.getByTestId("drop-send").click();
  await expect(alice.page.getByTestId("drop-ready")).toBeVisible({ timeout: 30_000 });
  const link = (await alice.page.getByTestId("drop-link").textContent()) ?? "";

  const live = mkdtempSync(join(tmpdir(), "hush-cli-"));
  const p2p = run(["receive", link, "--out", live, ...NET]);
  expect(await p2p.exited).toBe(0);
  expect(p2p.err.join("\n")).toContain("receiving peer to peer");
  expect(sha(readFileSync(join(live, "a.bin")))).toBe(sha(data));
  await expect(alice.page.getByText("Delivered directly")).toBeVisible();

  // Alice leaves: a second download comes from the sealed copy.
  await close(alice);
  const later = mkdtempSync(join(tmpdir(), "hush-cli-"));
  const copy = run(["receive", link, "--out", later, ...NET]);
  expect(await copy.exited).toBe(0);
  expect(copy.err.join("\n")).toContain("using the sealed copy");
  expect(sha(readFileSync(join(later, "a.bin")))).toBe(sha(data));
  expect(readFileSync(join(later, "b.txt"), "utf8")).toBe("b");
});

test("a backup exported by the web app opens in the CLI", async ({ browser }) => {
  const alice = await person(browser, "Alice");
  await open(alice);
  await alice.page.getByRole("button", { name: "My identifier" }).click();
  await alice.page.getByTestId("vault-pass").fill("a long vault passphrase");
  await alice.page.getByTestId("vault-confirm").fill("a long vault passphrase");
  await alice.page.getByRole("button", { name: "Create the vault" }).click();
  await alice.page.getByRole("button", { name: "Create my identifier" }).click();
  const id = (await alice.page.getByTestId("my-id").textContent()) ?? "";
  await alice.page.getByRole("button", { name: "Back", exact: true }).click();
  await alice.page.getByRole("button", { name: "Contacts" }).click();
  await alice.page.getByLabel("Backup passphrase").fill("backup passphrase");
  const download = alice.page.waitForEvent("download");
  await alice.page.getByRole("button", { name: "Export a backup" }).click();
  const file = await (await download).path();
  const ok = run(["backup", "inspect", file, "--passphrase", "backup passphrase"]);
  expect(await ok.exited).toBe(0);
  expect(ok.out.join("\n")).toContain(`identifier: ${id}`);
  const bad = run(["backup", "inspect", file, "--passphrase", "wrong"]);
  expect(await bad.exited).not.toBe(0);
  await close(alice);
});
