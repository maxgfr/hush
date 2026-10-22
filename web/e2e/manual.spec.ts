import { expect, type Page, test } from "@playwright/test";
import { close, createLink, open, person, sas, say } from "./helpers";

const OFFLINE = { nostr: false, torrent: false };

async function code(page: Page, previous = ""): Promise<string> {
  const area = page.getByTestId("manual-code");
  await expect(area).toBeVisible();
  await expect
    .poll(
      async () => (await area.inputValue()) !== previous && (await area.inputValue()).length > 0,
    )
    .toBe(true);
  return area.inputValue();
}

async function paste(page: Page, value: string): Promise<void> {
  await page.getByTestId("manual-input").fill(value);
  await page.getByRole("button", { name: "Join" }).click();
}

test("no network at all: the handshake travels as hand-carried codes", async ({ browser }) => {
  const alice = await person(browser, "Alice", OFFLINE);
  const bob = await person(browser, "Bob", OFFLINE);
  await open(alice);
  const link = await createLink(alice);
  await alice.page.getByRole("button", { name: "No Internet? Swap QR codes" }).click();
  await bob.page.goto(link);
  await bob.page.getByRole("button", { name: "No Internet? Swap QR codes" }).click();

  const hs1 = await code(bob.page);
  expect(hs1.startsWith("hushcode:")).toBe(true);
  await paste(alice.page, hs1);
  const hs2 = await code(alice.page);
  await paste(bob.page, hs2);
  await expect(bob.page.getByTestId("sas")).toBeVisible();
  const hs3 = await code(bob.page, hs1);
  await paste(alice.page, hs3);
  expect(await sas(alice)).toBe(await sas(bob));
  await say(alice, "offline hello");
  await expect(bob.page.getByText("offline hello")).toBeVisible();
  await close(alice, bob);
});

test("without a direct path, messages are relayed, still end-to-end encrypted", async ({
  browser,
}) => {
  // Relay-only ICE with no TURN server: WebRTC can never connect.
  const alice = await person(browser, "Alice", { relayOnly: true });
  const bob = await person(browser, "Bob", { relayOnly: true });
  await open(alice);
  const link = await createLink(alice);
  await bob.page.goto(link);
  await sas(bob);
  await expect(alice.page.getByTestId("peer-state")).toHaveAttribute("data-state", "relay", {
    timeout: 40_000,
  });
  await say(bob, "through the relays");
  await expect(alice.page.getByText("through the relays")).toBeVisible();
  await say(alice, "still private");
  await expect(bob.page.getByText("still private")).toBeVisible();
  await close(alice, bob);
});
