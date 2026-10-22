import { expect, type Page, test } from "@playwright/test";
import { close, createLink, open, type Person, person, sas, say } from "./helpers";

const PASS = "a long vault passphrase";

async function createVault(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Contacts" }).click();
  await page.getByTestId("vault-pass").fill(PASS);
  await page.getByTestId("vault-confirm").fill(PASS);
  await page.getByRole("button", { name: "Create the vault" }).click();
  await expect(page.getByRole("button", { name: "Lock" })).toBeVisible();
  await page.getByRole("button", { name: "Back", exact: true }).click();
}

async function verifiedPair(alice: Person, bob: Person): Promise<void> {
  const link = await createLink(alice);
  await bob.page.goto(link);
  expect(await sas(alice)).toBe(await sas(bob));
  await alice.page.getByRole("button", { name: "They match" }).click();
  await bob.page.getByRole("button", { name: "They match" }).click();
  await expect(alice.page.getByText("Verified")).toBeVisible();
}

test("save a contact, reconnect without a link, keep history, back up and restore", async ({
  browser,
}) => {
  const alice = await person(browser, "Alice");
  const bob = await person(browser, "Bob");
  await open(alice);
  await open(bob);
  await createVault(alice.page);
  await createVault(bob.page);
  await verifiedPair(alice, bob);

  await alice.page.getByRole("button", { name: "Menu" }).click();
  await alice.page.getByRole("menuitem", { name: "Save contact" }).click();
  await expect(alice.page.getByText("Request sent, waiting for Bob.")).toBeVisible();
  await expect(bob.page.getByText("Alice wants to save you as a contact.")).toBeVisible();
  await bob.page.getByRole("button", { name: "Save contact" }).click();
  await expect(alice.page.getByText("Bob is in your contacts.")).toBeVisible();
  await expect(bob.page.getByText("Alice is in your contacts.")).toBeVisible();

  await alice.page.getByTestId("end").click();
  // With an unlocked vault, "New conversation" goes home without reloading (contacts stay live).
  await alice.page.getByRole("button", { name: "New conversation" }).click();
  await alice.page.getByRole("button", { name: "Contacts" }).click();
  await bob.page.getByRole("button", { name: "New conversation" }).click();
  await bob.page.getByRole("button", { name: "Contacts" }).click();

  // Both are online: the contact handshake runs on its own.
  await expect(alice.page.getByText("online", { exact: true })).toBeVisible({ timeout: 60_000 });
  await alice.page.getByRole("button", { name: /Bob/ }).click();
  await say(alice, "back again, no link needed");
  await expect(bob.page.getByRole("button", { name: /Alice/ }).locator(".count")).toHaveText("1");
  await bob.page.getByRole("button", { name: /Alice/ }).click();
  await expect(bob.page.getByText("back again, no link needed")).toBeVisible();

  // Backup from Bob, restore into a fresh browser.
  await bob.page.getByRole("button", { name: "Back", exact: true }).click();
  await bob.page.getByLabel("Backup passphrase").fill("backup pass");
  const download = bob.page.waitForEvent("download");
  await bob.page.getByRole("button", { name: "Export a backup" }).click();
  const file = await (await download).path();

  const bob2 = await person(browser, "Bob");
  await open(bob2);
  await createVault(bob2.page);
  await bob2.page.getByRole("button", { name: "Contacts" }).click();
  await bob2.page.getByLabel("Backup passphrase").fill("backup pass");
  await bob2.page.locator('input[type="file"]').setInputFiles(file);
  await expect(bob2.page.getByText("Backup imported: 1 contacts.")).toBeVisible();
  await expect(bob2.page.getByRole("button", { name: /Alice/ })).toBeVisible();
  await close(alice, bob, bob2);
});

test("add someone by identifier, with their consent", async ({ browser }) => {
  const alice = await person(browser, "Alice");
  const bob = await person(browser, "Bob");
  await open(alice);
  await open(bob);
  await createVault(alice.page);
  await createVault(bob.page);
  await bob.page.getByRole("button", { name: "My identifier" }).click();
  await bob.page.getByRole("button", { name: "Create my identifier" }).click();
  const id = (await bob.page.getByTestId("my-id").textContent()) ?? "";
  expect(id).toMatch(/^hush1[a-z0-9]{59}$/);
  await bob.page.getByTestId("reachable").check();
  await bob.page.getByRole("button", { name: "Back", exact: true }).click();
  await bob.page.getByRole("button", { name: "Contacts" }).click();

  await alice.page.getByRole("button", { name: "My identifier" }).click();
  await alice.page.getByRole("button", { name: "Create my identifier" }).click();
  await alice.page.getByRole("button", { name: "Back", exact: true }).click();
  await alice.page.getByRole("button", { name: "Contacts" }).click();
  await alice.page.getByTestId("add-id").fill(`${id.slice(0, 20)}x${id.slice(21)}`);
  await alice.page.getByRole("button", { name: "Talk" }).click();
  await expect(alice.page.getByText("Invalid identifier (typo?).")).toBeVisible();
  await alice.page.getByTestId("add-id").fill(id);
  await alice.page.getByRole("button", { name: "Talk" }).click();
  await expect(bob.page.getByText(/Alice \(hush1/)).toBeVisible({ timeout: 60_000 });
  await bob.page.getByRole("button", { name: "Accept" }).click();
  await say(bob, "welcome");
  await expect(alice.page.getByText("welcome")).toBeVisible({ timeout: 60_000 });
  await close(alice, bob);
});

test("ending the conversation while a contact is being saved fails cleanly", async ({
  browser,
}) => {
  const alice = await person(browser, "Alice");
  const bob = await person(browser, "Bob");
  await open(alice);
  await open(bob);
  await createVault(alice.page);
  await createVault(bob.page);
  await verifiedPair(alice, bob);
  await alice.page.getByRole("button", { name: "Menu" }).click();
  await alice.page.getByRole("menuitem", { name: "Save contact" }).click();
  await expect(bob.page.getByText("Alice wants to save you as a contact.")).toBeVisible();
  // Alice leaves at the very moment Bob accepts.
  await Promise.all([
    alice.page.getByTestId("end").click(),
    // The button may already be gone if Alice's farewell wins the race: both outcomes are fine.
    bob.page
      .getByRole("button", { name: "Save contact" })
      .click({ timeout: 5000 })
      .catch(() => {}),
  ]);
  await expect(
    bob.page.getByRole("heading", { name: "The other person ended the conversation" }),
  ).toBeVisible();
  await bob.page.waitForTimeout(500);
  await close(alice, bob);
});
