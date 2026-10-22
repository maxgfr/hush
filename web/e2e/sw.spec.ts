import { expect, test } from "@playwright/test";
import { BASE } from "./helpers";

// Cache Storage is shared by every page of the origin (every maxgfr.github.io site): the service
// worker must never serve a cached file another page rewrote.
test("the service worker refuses a poisoned cache", async ({ browser }) => {
  const ctx = await browser.newContext({ locale: "en-US" });
  const page = await ctx.newPage();
  await page.goto(`${BASE}/`);
  await page.evaluate(() => navigator.serviceWorker.ready);
  await page.reload();
  await expect.poll(() => page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true);

  // Verified copies keep the app working offline.
  await ctx.setOffline(true);
  await page.reload();
  await expect(page.getByRole("button", { name: "New conversation" })).toBeVisible();
  await ctx.setOffline(false);

  // Another page of the origin rewrites the cached shell.
  await page.evaluate(async () => {
    const key = (await caches.keys()).find((k) => k.startsWith("hush-"));
    if (!key) throw new Error("no hush cache");
    const evil = "<!doctype html><title>pwned</title><p id=pwned>pwned</p>";
    const res = new Response(evil, { headers: { "content-type": "text/html" } });
    await (await caches.open(key)).put(new URL("./", location.href).href, res);
  });
  await page.reload();
  await expect(page).toHaveTitle("hush");
  await expect(page.locator("#pwned")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "New conversation" })).toBeVisible();
  // The tampered cache is gone.
  expect(
    await page.evaluate(async () => (await caches.keys()).some((k) => k.startsWith("hush-"))),
  ).toBe(false);
  await ctx.close();
});

// The body alone is checked, so the status and headers stored around it must never be served: a
// redirect around the genuine shell would carry the #link fragment to the attacker's page.
test("the service worker ignores a cached redirect around the genuine shell", async ({
  browser,
}) => {
  const ctx = await browser.newContext({ locale: "en-US" });
  const page = await ctx.newPage();
  await page.goto(`${BASE}/`);
  await page.evaluate(() => navigator.serviceWorker.ready);
  await page.reload();
  await expect.poll(() => page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true);

  await page.evaluate(async () => {
    const key = (await caches.keys()).find((k) => k.startsWith("hush-"));
    if (!key) throw new Error("no hush cache");
    const cache = await caches.open(key);
    const shell = new URL("./", location.href).href;
    const hit = await cache.match(shell);
    if (!hit) throw new Error("no cached shell");
    const genuine = await hit.arrayBuffer();
    const headers = {
      location: "/pwned/",
      refresh: "0; url=/pwned/",
      "content-type": "text/plain",
    };
    await cache.put(shell, new Response(genuine, { status: 302, headers }));
  });
  await page.goto(`${BASE}/#2.s.not-a-real-link`);
  await expect(page.getByRole("heading", { name: "Join with a link" })).toBeVisible();
  expect(new URL(page.url()).pathname).not.toContain("pwned");
  await page.reload();
  expect(new URL(page.url()).pathname).not.toContain("pwned");
  await expect(page).toHaveTitle("hush");
  await ctx.close();
});
