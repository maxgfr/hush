import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { expect, type Page, test } from "@playwright/test";
import { close, createLink, open, person, say } from "./helpers";

// The Rust engine the CLI and the mobile apps share, scripted: contacts and identifiers must work
// between a phone and a browser exactly as between two browsers.
const BIN = resolve(
  process.cwd(),
  "../target",
  process.env.HUSH_CLI_PROFILE ?? "debug",
  "examples/peer",
);
const PASS = "a long vault passphrase";

test.use({ launchOptions: { args: ["--disable-features=WebRtcHideLocalIpsWithMdns"] } });
test.skip(!existsSync(BIN), `build it first (cargo build -p hush-app --example peer): ${BIN}`);

interface Peer {
  child: ChildProcess;
  lines: string[];
  send(line: string): void;
  /** The first line matching, once it is printed. */
  line(re: RegExp, timeout?: number): Promise<RegExpMatchArray>;
}

function robot(): Peer {
  const data = mkdtempSync(join(tmpdir(), "hush-peer-"));
  const child = spawn(
    BIN,
    ["--relay", "ws://localhost:4173/relay", "--data", data, "--name", "Robot"],
    {
      stdio: ["pipe", "pipe", "inherit"],
    },
  );
  const lines: string[] = [];
  child.stdout?.on("data", (d: Buffer) => {
    if (process.env.HUSH_E2E_VERBOSE) process.stderr.write(`[peer] ${d}`);
    lines.push(...d.toString().split("\n").filter(Boolean));
  });
  return {
    child,
    lines,
    send: (l) => child.stdin?.write(`${l}\n`),
    async line(re, timeout = 60_000) {
      let hit: RegExpMatchArray | null = null;
      await expect
        .poll(
          () => {
            for (const l of lines) {
              hit = l.match(re);
              if (hit) return true;
            }
            return false;
          },
          { timeout, message: `${re} in:\n${lines.join("\n")}` },
        )
        .toBe(true);
      return hit as unknown as RegExpMatchArray;
    },
  };
}

async function createVault(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Contacts" }).click();
  await page.getByTestId("vault-pass").fill(PASS);
  await page.getByTestId("vault-confirm").fill(PASS);
  await page.getByRole("button", { name: "Create the vault" }).click();
  await expect(page.getByRole("button", { name: "Lock" })).toBeVisible();
  await page.getByRole("button", { name: "Back", exact: true }).click();
}

test("a browser and the Rust engine save each other, then reconnect without a link", async ({
  browser,
}) => {
  const alice = await person(browser, "Alice");
  await open(alice);
  await createVault(alice.page);
  const r = robot();
  r.send(`vault ${PASS}`);
  await r.line(/^vault ok/);

  const link = await createLink(alice);
  r.send(`join ${link}`);
  const chat = (await r.line(/^joined (\d+)/))[1];
  const peer = (await r.line(new RegExp(`^peer-joined ${chat} ([0-9a-f]+) Alice`)))[1];
  await alice.page.getByRole("button", { name: "They match" }).click();
  r.send(`verify ${chat}`);
  await expect(alice.page.getByText("Verified")).toBeVisible();
  await r.line(new RegExp(`^verified ${chat} true true`));

  await alice.page.getByRole("button", { name: "Menu" }).click();
  await alice.page.getByRole("menuitem", { name: "Save contact" }).click();
  await r.line(new RegExp(`^card-offer ${chat} ${peer} Alice`));
  r.send(`save ${chat} ${peer}`);
  await expect(alice.page.getByText("Robot is in your contacts.")).toBeVisible();
  await r.line(/ContactSaved \{ name: "Alice" \}/);

  await alice.page.getByTestId("end").click();
  await r.line(new RegExp(`^ended ${chat}`));
  await alice.page.getByRole("button", { name: "New conversation" }).click();
  await alice.page.getByRole("button", { name: "Contacts" }).click();
  // Both online: the contact handshake runs by itself.
  await expect(alice.page.getByText("online", { exact: true })).toBeVisible({ timeout: 60_000 });
  const contactChat = (await r.line(/^chat (\d+) Contact/))[1];
  await r.line(new RegExp(`^peer-joined ${contactChat} [0-9a-f]+ Alice`));
  await alice.page.getByRole("button", { name: /Robot/ }).click();
  await say(alice, "back again, no link needed");
  await r.line(new RegExp(`^message ${contactChat} Alice: back again, no link needed`));
  r.send(`send ${contactChat} welcome back`);
  await expect(alice.page.getByText("welcome back")).toBeVisible();
  r.child.kill();
  await close(alice);
});

test("a browser asks the Rust engine by identifier", async ({ browser }) => {
  const r = robot();
  r.send(`vault ${PASS}`);
  await r.line(/^vault ok/);
  r.send("identity");
  const id = (await r.line(/^identity (hush1[a-z0-9]+)/))[1];
  r.send("reachable");
  await r.line(/^Reachable\(true\)/);

  const alice = await person(browser, "Alice");
  await open(alice);
  await createVault(alice.page);
  await alice.page.getByRole("button", { name: "My identifier" }).click();
  await alice.page.getByRole("button", { name: "Create my identifier" }).click();
  const aliceId = (await alice.page.getByTestId("my-id").textContent()) ?? "";
  await alice.page.getByRole("button", { name: "Back", exact: true }).click();
  await alice.page.getByRole("button", { name: "Contacts" }).click();
  await alice.page.getByTestId("add-id").fill(id);
  await alice.page.getByRole("button", { name: "Talk" }).click();

  await r.line(new RegExp(`^requests .*${aliceId}=Alice`));
  r.send(`accept ${aliceId}`);
  const chat = (await r.line(/^peer-joined (\d+) [0-9a-f]+ Alice/))[1];
  r.send(`send ${chat} hello from Rust`);
  await expect(alice.page.getByText("hello from Rust")).toBeVisible({ timeout: 60_000 });
  await say(alice, "hello from the browser");
  await r.line(new RegExp(`^message ${chat} Alice: hello from the browser`));
  // Both saved each other, as two browsers would.
  await r.line(/ContactSaved \{ name: "Alice" \}/);
  r.child.kill();
  await close(alice);
});
