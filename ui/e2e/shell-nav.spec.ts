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

  // Branding: ONE logo anchor, hosted by the sidebar (the navbar shows the
  // breadcrumb instead — the duplicate-logo regression guard).
  await expect(page.getByTestId("brand").locator("img")).toBeVisible();
  await expect(page.getByTestId("brand")).toHaveCount(1);
  await expect(page.getByTestId("sidebar").getByTestId("brand")).toBeVisible();
  await expect(page.getByTestId("navbar").getByTestId("brand")).toHaveCount(0);
  await expect(page.locator('link[rel="icon"]')).toHaveCount(1);

  const sections: Array<[string, string, string]> = [
    ["nav-chat", "chat-panel", "Chat"],
    ["nav-runs", "runs-section", "Runs"],
    ["nav-recipes", "recipes-section", "Blueprints"],
    ["nav-artifacts", "artifacts-section", "Artifacts"],
    ["nav-datasets", "datasets-section", "Datasets"],
    ["nav-systems", "systems-section", "Systems"],
    ["nav-settings", "settings-section", "Settings"],
    ["nav-activity", "activity-panel", "Activity"],
  ];
  for (const [nav, panel, crumb] of sections) {
    await page.getByTestId(nav).click();
    await expect(page.getByTestId(panel)).toBeVisible({ timeout: 15_000 });
    // The navbar breadcrumb tracks the active section.
    await expect(page.getByTestId("breadcrumb")).toContainText(crumb);
  }
});
