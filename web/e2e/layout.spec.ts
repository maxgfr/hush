import { expect, test } from "@playwright/test";
import { createLink, open, person, sas } from "./helpers";

test("phones: no horizontal scroll on any screen, tap targets reachable", async ({ browser }) => {
  const alice = await person(browser, "Alice");
  await alice.page.setViewportSize({ width: 360, height: 740 });
  const noOverflow = async () =>
    expect(
      await alice.page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth + 1,
      ),
    ).toBe(true);
  await open(alice);
  await noOverflow();
  const link = await createLink(alice);
  await noOverflow();
  const bob = await person(browser, "Bob");
  await bob.page.setViewportSize({ width: 360, height: 740 });
  await bob.page.goto(link);
  await sas(bob);
  expect(
    await bob.page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1),
  ).toBe(true);
  await expect(bob.page.getByTestId("end")).toBeInViewport();
  await expect(bob.page.getByTestId("composer")).toBeInViewport();
});
