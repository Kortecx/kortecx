import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("the app shell navigates to every section (brand + favicon present)", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Branding: the kortecx icon + a favicon link.
  await expect(page.getByTestId("brand").locator("img")).toBeVisible();
  await expect(page.locator('link[rel="icon"]')).toHaveCount(1);

  const sections: Array<[string, string]> = [
    ["nav-chat", "chat-panel"],
    ["nav-runs", "runs-section"],
    ["nav-recipes", "recipes-section"],
    ["nav-artifacts", "artifacts-section"],
    ["nav-datasets", "datasets-section"],
    ["nav-systems", "systems-section"],
    ["nav-activity", "activity-panel"],
  ];
  for (const [nav, panel] of sections) {
    await page.getByTestId(nav).click();
    await expect(page.getByTestId(panel)).toBeVisible({ timeout: 15_000 });
  }
});
