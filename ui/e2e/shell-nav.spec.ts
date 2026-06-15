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

  // The spec-IA sections in the spec's order (+ pinned Settings); the Dashboard
  // landing is exercised separately by dashboard.spec.ts.
  const sections: Array<[string, string, string]> = [
    ["nav-chat", "chat-panel", "New Chat"],
    ["nav-runs", "runs-section", "Workflows"],
    ["nav-recipes", "recipes-section", "Blueprints"],
    ["nav-datasets", "datasets-section", "Datasets"],
    ["nav-tools", "tools-section", "Tools"],
    ["nav-models", "models-section", "Models"],
    ["nav-context", "context-section", "Context"],
    ["nav-monitor", "monitoring-section", "Monitoring"],
    ["nav-systems", "systems-section", "Security"],
    ["nav-settings", "settings-section", "Settings"],
  ];
  for (const [nav, panel, crumb] of sections) {
    await page.getByTestId(nav).click();
    await expect(page.getByTestId(panel)).toBeVisible({ timeout: 15_000 });
    // The navbar breadcrumb tracks the active section.
    await expect(page.getByTestId("breadcrumb")).toContainText(crumb);
  }

  // Activity is a navbar drawer (the spec's top-bar control), not a section.
  await expect(page.getByTestId("nav-activity")).toHaveCount(0);
  await page.getByTestId("activity-toggle").click();
  await expect(page.getByTestId("activity-drawer")).toBeVisible();
  await expect(page.getByTestId("activity-panel")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("activity-close").click();
  await expect(page.getByTestId("activity-drawer")).toHaveCount(0);

  // The navbar hosts the quick theme switch next to the controls.
  await page.getByTestId("theme-toggle").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByTestId("theme-toggle").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("the sidebar groups sections + a New flyout; Cloud is honest-disabled (PR-B)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // The five coloured groups + the honest Cloud group render their labels.
  for (const g of ["workspace", "data", "tools", "monitoring", "security", "cloud"]) {
    await expect(page.getByTestId(`nav-group-${g}`)).toBeVisible();
  }
  await expect(page.getByTestId("nav-group-workspace")).toContainText("Workspace");

  // The topbar system-status box reports REAL health (live on a reachable serve).
  await expect(page.getByTestId("system-status")).toBeVisible();
  await expect(page.getByTestId("system-status")).toHaveAttribute("data-health", "live");

  // The New flyout opens (below the trigger) and routes to a real target.
  await page.getByTestId("sidebar-new").click();
  await expect(page.getByTestId("sidebar-new-menu")).toBeVisible();
  await expect(page.getByTestId("new-blueprint")).toBeVisible();
  await page.getByTestId("new-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();

  // Cloud placeholders render but are NEVER navigable (greyed, aria-disabled, no link).
  const cloud = page.getByTestId("cloud-sharing");
  await expect(cloud).toBeVisible();
  await expect(cloud).toHaveAttribute("aria-disabled", "true");
  expect(await cloud.evaluate((el) => el.tagName)).toBe("DIV");
});
