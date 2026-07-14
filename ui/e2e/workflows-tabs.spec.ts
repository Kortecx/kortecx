/**
 * The /workflows section: three tabs (Catalog / Runs / Templates), a top-right
 * Refresh + New-workflow cluster, and a high-level catalog of workflow CARDS
 * (name · description · Run / Schedule / Share + a kebab). This pins the tab
 * structure, the card action affordances (run form, schedule popover, edit-in-
 * builder), the Templates placeholder, the top-right actions, and a dark-theme
 * render. (Populated Runs-table assertions live in runs-list.spec.ts.)
 */

import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { expectOverlayAboveNavbar } from "./fixtures/overlay";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("workflows catalog: high-level cards with Run form, Schedule, and Edit-in-builder", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("runs-section")).toBeVisible();

  // Three tabs; the catalog is default and renders a CARD GRID (not a raw table).
  await expect(page.getByTestId("workflows-tabs")).toBeVisible();
  await expect(page.getByTestId("workflows-tab-catalog")).toBeVisible();
  await expect(page.getByTestId("workflows-tab-runs")).toBeVisible();
  await expect(page.getByTestId("workflows-tab-templates")).toBeVisible();
  await expect(page.getByTestId("workflows-catalog")).toBeVisible({ timeout: 30_000 });

  // The echo workflow shows as a card with its clean name — no raw handle on the card.
  const card = page
    .getByTestId("workflow-card")
    .filter({ has: page.getByTestId("workflow-open-kx/recipes/echo") });
  await expect(card).toBeVisible();
  await expect(card).not.toContainText("kx/recipes/echo");

  // Run → the input form drawer opens (echo has a `topic` param); Escape closes it.
  await card.getByTestId("workflow-run-kx/recipes/echo").click();
  await expect(page.getByTestId("blueprint-form-drawer")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("field-topic")).toBeVisible({ timeout: 30_000 });
  // The run-form drawer is a portaled full-viewport overlay ABOVE the sticky navbar
  // (not clipped) — an occlusion proof `toBeVisible` can't make.
  await expectOverlayAboveNavbar(page, "blueprint-form-drawer");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("blueprint-form-drawer")).toBeHidden();

  // Schedule → the local CRON trigger popover (no cloud).
  await card.getByTestId("workflow-schedule-kx/recipes/echo").click();
  await expect(page.getByTestId("schedule-name")).toBeVisible();
  await page.keyboard.press("Escape");

  // Kebab → Edit in builder links to the visual builder; Open-in-new-tab is a new window.
  await card.getByTestId("workflow-menu-kx/recipes/echo").click();
  await expect(card.getByTestId("workflow-edit")).toHaveAttribute("href", /\/blueprints\/new/);
  await expect(card.getByTestId("workflow-open-newtab")).toHaveAttribute("target", "_blank");
});

test("workflows: the top-right Refresh + New-workflow actions", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("workflows-catalog")).toBeVisible({ timeout: 30_000 });

  // Refresh re-pulls the catalog without error (the grid stays visible).
  await page.getByTestId("workflows-refresh").click();
  await expect(page.getByTestId("workflows-catalog")).toBeVisible({ timeout: 30_000 });

  // New workflow opens the visual builder.
  await page.getByTestId("workflows-new").click();
  await expect(page).toHaveURL(/\/blueprints\/new/);
  await expect(page.getByTestId("builder-canvas")).toBeVisible({ timeout: 30_000 });
});

test("workflows: the Templates tab shows the honest 'coming next' placeholder", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-runs").click();
  await page.getByTestId("workflows-tab-templates").click();
  await expect(page.getByTestId("workflows-templates")).toBeVisible();
  await expect(page.getByTestId("workflows-templates-placeholder")).toBeVisible();
  await expect(page.getByTestId("workflows-catalog")).toHaveCount(0);
});

test("run history is reachable from the Workflows → Runs tab", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("runs-section")).toBeVisible();
  await page.getByTestId("workflows-tab-runs").click();
  await expect(page.getByTestId("workflows-runs")).toBeVisible({ timeout: 15_000 });
});

test("workflows section renders under the dark theme", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  const html = page.locator("html");
  for (let i = 0; i < 3 && (await html.getAttribute("data-theme")) !== "dark"; i++) {
    await page.getByTestId("theme-toggle").click();
  }
  await expect(html).toHaveAttribute("data-theme", "dark");

  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("workflows-catalog")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("workflow-card").first()).toBeVisible();

  // The run-form drawer is an un-clipped full-viewport overlay in DARK too (both-theme
  // geometry — the fix must not regress under either palette).
  const darkCard = page
    .getByTestId("workflow-card")
    .filter({ has: page.getByTestId("workflow-open-kx/recipes/echo") });
  await darkCard.getByTestId("workflow-run-kx/recipes/echo").click();
  await expect(page.getByTestId("blueprint-form-drawer")).toBeVisible({ timeout: 30_000 });
  await expectOverlayAboveNavbar(page, "blueprint-form-drawer");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("blueprint-form-drawer")).toBeHidden();

  await page.getByTestId("workflows-tab-templates").click();
  await expect(page.getByTestId("workflows-templates-placeholder")).toBeVisible();
});
