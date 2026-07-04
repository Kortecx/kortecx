import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("monitoring/approvals: URL-addressable tab, honest empty inbox, no false nav badge, both themes", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // A healthy run that COMMITS — it does not stage a world-mutating action, so it
  // never produces a pending approval (GR15: the inbox shows only real withheld
  // actions, never a fabricated row).
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "ok" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // No pending approvals ⇒ the Monitoring nav item carries NO badge (a false badge
  // would misreport that an autonomous run is blocked awaiting a decision).
  await expect(page.getByTestId("nav-badge-monitor")).toHaveCount(0);

  await page.getByTestId("nav-monitor").click();
  await expect(page.getByTestId("monitoring-section")).toBeVisible({ timeout: 15_000 });

  // The Approvals tab is URL-addressable (the run-detail tab precedent).
  await page.getByTestId("monitor-tab-approvals").click();
  await expect(page).toHaveURL(/\/monitor\?tab=approvals$/);
  await expect(page.getByTestId("monitor-tab-approvals")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("monitor-approvals")).toBeVisible();

  // A serve with the approval admin wired but nothing withheld ⇒ the honest
  // actionable empty state (NOT the not-wired note, NOT a fabricated row).
  await expect(page.getByText(/No actions awaiting approval/).first()).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByTestId("approval-row")).toHaveCount(0);
  await expect(page.getByTestId("approvals-not-wired")).toHaveCount(0);

  // BOTH THEMES (D142.1/GR13): the touched view renders under the dark palette.
  await page.getByTestId("theme-toggle").click();
  await expect(page.getByText(/No actions awaiting approval/).first()).toBeVisible();

  // Tab click rewrites the URL back to overview (drops the param).
  await page.getByTestId("monitor-tab-overview").click();
  await expect(page).toHaveURL(/\/monitor$/);
});
