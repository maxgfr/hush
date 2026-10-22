import { expect, test } from "@playwright/test";
import { close, createLink, open, person, sas, say } from "./helpers";

test("a one-to-one link: verify, chat, send a file, end", async ({ browser }) => {
  const alice = await person(browser, "Alice");
  const bob = await person(browser, "Bob");
  await open(alice);
  const link = await createLink(alice);
  expect(link).toMatch(/#1\.s\.[A-Za-z0-9_-]{43}\.[0-9a-z]+$/);

  await bob.page.goto(link);
  // The link leaves the address bar as soon as it is read.
  await expect.poll(() => new URL(bob.page.url()).hash).toBe("");

  const [a, b] = [await sas(alice), await sas(bob)];
  expect(a).toBe(b);
  await alice.page.getByRole("button", { name: "They match" }).click();
  await bob.page.getByRole("button", { name: "They match" }).click();
  await expect(alice.page.getByText("Verified")).toBeVisible();
  await expect(bob.page.getByText("Verified")).toBeVisible();
  await expect(alice.page.getByRole("heading", { name: "Bob" })).toBeVisible();

  await say(alice, "hello from alice");
  await expect(bob.page.getByText("hello from alice")).toBeVisible();
  await say(bob, "hi alice");
  await expect(alice.page.getByText("hi alice")).toBeVisible();

  await alice.page.locator('input[type="file"]').setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("x".repeat(200_000)),
  });
  await expect(bob.page.getByText("notes.txt")).toBeVisible();
  await bob.page.getByRole("button", { name: "Download" }).click();
  const dl = bob.page.waitForEvent("download");
  await bob.page.locator("a[download='notes.txt']").click();
  expect((await dl).suggestedFilename()).toBe("notes.txt");

  await alice.page.getByTestId("end").click();
  await expect(alice.page.getByRole("heading", { name: "Conversation ended" })).toBeVisible();
  await expect(
    bob.page.getByRole("heading", { name: "The other person ended the conversation" }),
  ).toBeVisible();
  await close(alice, bob);
});

test("a one-to-one link cannot be used twice", async ({ browser }) => {
  const alice = await person(browser, "Alice");
  const bob = await person(browser, "Bob");
  const eve = await person(browser, "Eve");
  await open(alice);
  const link = await createLink(alice);
  await bob.page.goto(link);
  await sas(bob);
  await eve.page.goto(link);
  await expect(eve.page.getByRole("heading", { name: "This link was already used" })).toBeVisible();
  await close(alice, bob, eve);
});

test("a passphrase protects the link", async ({ browser }) => {
  const alice = await person(browser, "Alice");
  const bob = await person(browser, "Bob");
  await open(alice);
  const link = await createLink(alice, { passphrase: "correct horse battery" });
  await bob.page.goto(link);
  await expect(
    bob.page.getByRole("heading", { name: "This link is protected by a passphrase" }),
  ).toBeVisible();
  await bob.page.getByLabel("Passphrase").fill("correct horse battery");
  await bob.page.getByRole("button", { name: "Join" }).click();
  expect(await sas(bob)).toBe(await sas(alice));
  await close(alice, bob);
});
