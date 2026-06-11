import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("the DevTools dock opens lazily, shows gateway health, and tails run events", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // Closed by default; the navbar toggle lazy-loads + opens the dock.
  await expect(page.getByTestId("devtools-dock")).toHaveCount(0);
  await page.getByTestId("devtools-toggle").click();
  await expect(page.getByTestId("devtools-dock")).toBeVisible({ timeout: 15_000 });

  // Health tab: the gateway probe + the connection profile.
  await page.getByTestId("devtools-tab-health").click();
  await expect(page.getByTestId("devtools-health")).toContainText("live", { timeout: 15_000 });
  await expect(page.getByTestId("devtools-health")).toContainText(gw.endpoint);

  // Events tab: empty until a run exists, then tails the latest run's deltas.
  await page.getByTestId("devtools-tab-events").click();
  await expect(page.getByTestId("devtools-no-run")).toBeVisible();

  await runRecipe(page);
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("devtools-events")).toBeVisible();
  await expect(page.getByTestId("devtools-dock").getByTestId("event-row").first()).toBeVisible({
    timeout: 30_000,
  });

  // Close restores the chrome.
  await page.getByTestId("devtools-close").click();
  await expect(page.getByTestId("devtools-dock")).toHaveCount(0);
});
