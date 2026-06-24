import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

/** The eight flat sections (POC-5c / D168), in nav order, with their panel + crumb. */
const SECTIONS: Array<[string, string, string]> = [
  ["nav-chat", "chat-panel", "New Chat"],
  ["nav-apps", "apps-section", "Apps"],
  ["nav-runs", "runs-section", "Workflows"],
  ["nav-context", "context-section", "Context"],
  ["nav-tools", "tools-section", "Tools"],
  ["nav-models", "models-section", "Models"],
  ["nav-monitor", "monitoring-section", "Monitoring"],
  ["nav-systems", "systems-section", "Security"],
];

test("the app shell navigates to every flat section (brand + favicon present)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Branding: ONE logo anchor, hosted by the sidebar (the navbar shows the
  // breadcrumb instead — the duplicate-logo regression guard).
  await expect(page.getByTestId("brand").locator("img")).toBeVisible();
  await expect(page.getByTestId("brand")).toHaveCount(1);
  await expect(page.getByTestId("sidebar").getByTestId("brand")).toBeVisible();
  await expect(page.getByTestId("navbar").getByTestId("brand")).toHaveCount(0);
  await expect(page.locator('link[rel="icon"]')).toHaveCount(1);

  for (const [nav, panel, crumb] of SECTIONS) {
    await page.getByTestId(nav).click();
    await expect(page.getByTestId(panel)).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("breadcrumb")).toContainText(crumb);
  }
  // Settings is pinned shell chrome (not in the flat list).
  await page.getByTestId("nav-settings").click();
  await expect(page.getByTestId("settings-section")).toBeVisible({ timeout: 15_000 });

  // Activity is a navbar drawer (the spec's top-bar control), not a section.
  await expect(page.getByTestId("nav-activity")).toHaveCount(0);
  await page.getByTestId("activity-toggle").click();
  await expect(page.getByTestId("activity-drawer")).toBeVisible();
  await expect(page.getByTestId("activity-panel")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("activity-close").click();
  await expect(page.getByTestId("activity-drawer")).toHaveCount(0);
});

test("the sidebar is a FLAT list — no groups, no Cloud/Coming, demoted routes via ⌘K", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // No coloured groups, no honest-Cloud group, no in-dev placeholders (POC-5c).
  for (const g of ["workspace", "data", "tools", "monitoring", "security", "cloud", "dev"]) {
    await expect(page.getByTestId(`nav-group-${g}`)).toHaveCount(0);
  }
  await expect(page.getByTestId("cloud-sharing")).toHaveCount(0);
  // The five demoted sections are NOT sidebar buttons.
  for (const id of ["dashboard", "recipes", "datasets", "branches", "policies"]) {
    await expect(page.getByTestId(`nav-${id}`)).toHaveCount(0);
  }

  // The topbar system-status box reports REAL health (live on a reachable serve).
  await expect(page.getByTestId("system-status")).toBeVisible();
  await expect(page.getByTestId("system-status")).toHaveAttribute("data-health", "live");

  // The New flyout opens and routes to a real target.
  await page.getByTestId("sidebar-new").click();
  await expect(page.getByTestId("sidebar-new-menu")).toBeVisible();
  await expect(page.getByTestId("new-blueprint")).toBeVisible();
  await page.getByTestId("new-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();

  // The demoted sections stay reachable (NO capability regression) via the ⌘K palette.
  await gotoViaPalette(page, "recipes");
  await expect(page.getByTestId("recipes-section")).toBeVisible({ timeout: 15_000 });
  await gotoViaPalette(page, "policies");
  await expect(page.getByTestId("policies-section")).toBeVisible({ timeout: 15_000 });
});

test("every flat section renders under BOTH themes (D142 / GR13)", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  for (const theme of ["light", "dark"] as const) {
    // The navbar hosts the quick theme switch; toggle to the target theme.
    const current = await page.locator("html").getAttribute("data-theme");
    if (current !== theme) {
      await page.getByTestId("theme-toggle").click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    for (const [nav, panel] of SECTIONS) {
      await page.getByTestId(nav).click();
      await expect(page.getByTestId(panel)).toBeVisible({ timeout: 15_000 });
    }
  }
});
