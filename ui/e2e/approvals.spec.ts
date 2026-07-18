import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { expectOverlayAboveNavbar } from "./fixtures/overlay";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("approvals bell: no false badge, drawer honest empty state, both themes", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // A healthy run that COMMITS but stages no world-mutating action ⇒ never a pending
  // approval (GR15: the bell counts only real withheld actions, never a fabricated one).
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "ok" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // No pending approvals ⇒ the navbar bell carries NO badge (a false badge would
  // misreport that an autonomous run is blocked awaiting a decision).
  await expect(page.getByTestId("approvals-bell")).toBeVisible();
  await expect(page.getByTestId("nav-badge-approvals")).toHaveCount(0);

  // Click the bell ⇒ the right-side approvals drawer opens ABOVE the sticky navbar.
  await page.getByTestId("approvals-bell").click();
  await expect(page.getByTestId("approvals-drawer")).toBeVisible();
  await expectOverlayAboveNavbar(page, "approvals-drawer");

  // A serve with the approval admin wired but nothing withheld ⇒ the honest actionable
  // empty state (NOT the not-wired note, NOT a fabricated row).
  await expect(page.getByText(/No actions awaiting approval/).first()).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByTestId("approval-row")).toHaveCount(0);
  await expect(page.getByTestId("approvals-not-wired")).toHaveCount(0);

  // Escape closes the drawer (the full-viewport scrim otherwise intercepts navbar clicks).
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("approvals-drawer")).toHaveCount(0);

  // BOTH THEMES (D142.1/GR13): the drawer renders under the dark palette too.
  await page.getByTestId("theme-toggle").click();
  await page.getByTestId("approvals-bell").click();
  await expect(page.getByTestId("approvals-drawer")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("approvals-drawer")).toHaveCount(0);
});
