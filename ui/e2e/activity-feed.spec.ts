import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("Activity feed tails events over the WS bridge (real browser WebSocket)", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  // Connect with the explicit WS-bridge endpoint (random port per fixture).
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // Submit an echo run and let it commit.
  await runRecipe(page);
  await expect(page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).first()).toBeVisible(
    {
      timeout: 30_000,
    },
  );

  // Open the run in the navbar ACTIVITY DRAWER (in-app, keeps the connection); the
  // feed connects to the WS bridge (since=0 replays the run's committed delta) and
  // renders an event row.
  const m = page.url().match(/\/runs\/([0-9a-f]{32})/);
  const instance = m?.[1] ?? "";
  await page.getByTestId("activity-toggle").click();
  await expect(page.getByTestId("activity-drawer")).toBeVisible();
  await expect(page.getByTestId("activity-panel")).toBeVisible();
  await page.getByTestId("run-picker").locator("select").selectOption(instance);

  await expect(page.getByTestId("event-row").first()).toBeVisible({ timeout: 30_000 });
});
