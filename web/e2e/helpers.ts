import { type Browser, type BrowserContext, expect, type Page } from "@playwright/test";

export const BASE = "http://localhost:4173";

/** Local relay only, no tracker, no STUN: tests never touch the public network. */
export function settingsJson(name: string, extra: Record<string, unknown> = {}): string {
  return JSON.stringify({
    name,
    relays: ["ws://localhost:4173/relay"],
    trackers: ["ws://localhost:4173/tracker"],
    nostr: true,
    torrent: false,
    stun: [],
    turn: null,
    relayOnly: false,
    ttl: 0,
    ...extra,
  });
}

export interface Person {
  ctx: BrowserContext;
  page: Page;
  errors: string[];
}

export async function person(browser: Browser, name: string, extra: Record<string, unknown> = {}): Promise<Person> {
  const ctx = await browser.newContext({ acceptDownloads: true, locale: "en-US" });
  await ctx.addInitScript(
    ({ s }) => {
      if (sessionStorage.getItem("hush.test.seeded")) return;
      localStorage.setItem("hush.settings.v1", s);
      localStorage.setItem("hush.lang", "en");
      sessionStorage.setItem("hush.test.seeded", "1");
    },
    { s: settingsJson(name, extra) },
  );
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("dialog", (d) => void d.accept());
  if (process.env.HUSH_E2E_VERBOSE) page.on("console", (m) => console.log(`[${name}] ${m.text()}`));
  return { ctx, page, errors };
}

/** Opens the app (no service worker, so every run tests the fresh build). */
export async function open(p: Person, hash = ""): Promise<void> {
  await p.page.goto(`${BASE}/?nosw${hash}`);
  await expect(p.page.getByRole("heading", { level: 1 }).first()).toBeVisible();
}

/** Creates a link from the home screen and returns it. */
export async function createLink(p: Person, opts: { group?: boolean; passphrase?: string } = {}): Promise<string> {
  await p.page.getByRole("button", { name: "New conversation" }).click();
  if (opts.group) await p.page.getByText("Group", { exact: true }).click();
  if (opts.passphrase) await p.page.getByLabel("Passphrase (optional)").fill(opts.passphrase);
  await p.page.getByRole("button", { name: "Create the link" }).click();
  const link = p.page.getByTestId("share-link");
  await expect(link).toBeVisible();
  return (await link.textContent()) ?? "";
}

export async function sas(p: Person): Promise<string> {
  const el = p.page.getByTestId("sas").first();
  await expect(el).toBeVisible();
  return (await el.textContent()) ?? "";
}

export async function say(p: Person, text: string): Promise<void> {
  await p.page.getByTestId("composer").fill(text);
  await p.page.getByTestId("composer").press("Enter");
}

export async function close(...people: Person[]): Promise<void> {
  for (const p of people) {
    expect(p.errors, "no uncaught errors").toEqual([]);
    await p.ctx.close();
  }
}
