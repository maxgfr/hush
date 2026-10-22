import { expect, test } from "@playwright/test";
import { close, createLink, open, person, say } from "./helpers";

test("a group link: three people, everyone sees everyone, locking keeps others out", async ({
  browser,
}) => {
  const alice = await person(browser, "Alice");
  const bob = await person(browser, "Bob");
  const carol = await person(browser, "Carol");
  const dave = await person(browser, "Dave");
  await open(alice);
  const link = await createLink(alice, { group: true });
  expect(link).toContain("#2.g.");
  await bob.page.goto(link);
  await expect(bob.page.getByRole("heading", { name: "2 people" })).toBeVisible();
  await carol.page.goto(link);
  // Carol connects to both Alice and Bob (full mesh).
  await expect(carol.page.getByRole("heading", { name: "3 people" })).toBeVisible();
  await expect(alice.page.getByRole("heading", { name: "3 people" })).toBeVisible();
  await expect(bob.page.getByRole("heading", { name: "3 people" })).toBeVisible();

  await say(carol, "hi all");
  await expect(alice.page.getByText("hi all")).toBeVisible();
  await expect(bob.page.getByText("hi all")).toBeVisible();
  await say(bob, "hey carol");
  await expect(carol.page.getByText("hey carol")).toBeVisible();

  await alice.page.getByRole("button", { name: "Menu" }).click();
  await alice.page.getByRole("menuitem", { name: "Lock the room" }).click();
  await expect(bob.page.getByText("Room locked: nobody else can join.")).toBeVisible();
  await dave.page.goto(link);
  await dave.page.waitForTimeout(3000);
  await expect(dave.page.getByTestId("status")).toBeVisible();
  await expect(alice.page.getByRole("heading", { name: "3 people" })).toBeVisible();

  await carol.page.getByTestId("end").click();
  await expect(alice.page.getByRole("heading", { name: "2 people" })).toBeVisible();
  await close(alice, bob, carol, dave);
});
