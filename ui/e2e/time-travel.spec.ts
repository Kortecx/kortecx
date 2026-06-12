import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("Activity time-travel: scrub a run back to seq 0 and resume live", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Submit an echo run and let it commit.
  await runRecipe(page);
  await expect(page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).first()).toBeVisible(
    {
      timeout: 30_000,
    },
  );

  // Grab the instance id, then open the navbar ACTIVITY DRAWER (in-app — a reload
  // would drop the in-memory connection) and select the run.
  const m = page.url().match(/\/runs\/([0-9a-f]{32})/);
  expect(m).not.toBeNull();
  const instance = m?.[1] ?? "";
  await page.getByTestId("activity-toggle").click();
  await expect(page.getByTestId("activity-panel")).toBeVisible();
  await page.getByTestId("run-picker").locator("select").selectOption(instance);

  await expect(page.getByTestId("metrics-panel")).toBeVisible({ timeout: 30_000 });
  const scrubber = page.getByTestId("time-travel");
  await expect(scrubber).toBeVisible({ timeout: 30_000 });

  // Pin to seq 0 (the empty/initial frontier), then resume live.
  await page.getByLabel(/journal sequence/i).fill("0");
  await expect(page.getByTestId("scrubber-seq")).toHaveText("#0");
  await page.getByTestId("scrubber-live").click();
  await expect(page.getByTestId("scrubber-seq")).toHaveText("live");
});
