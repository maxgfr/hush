import { expect, test } from "@playwright/test";
import { open, person } from "./helpers";

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
