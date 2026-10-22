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
