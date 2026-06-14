import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("Systems: the seeded demo team, its members + roles, and the grants inspector", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-systems").click();
  await expect(page.getByTestId("systems-section")).toBeVisible();
  await expect(page.getByTestId("teams-panel")).toBeVisible();

  // The bootstrap-seeded demo team renders as a CHIP (a button, NOT a controlled
  // <select> — the Playwright selectOption gotcha). Selecting it drives the member table.
  const teamChip = page.getByTestId("team-pick-kx/teams/workspace");
  await expect(teamChip).toBeVisible({ timeout: 30_000 });
  await expect(teamChip).toHaveAttribute("aria-pressed", "true"); // auto-selected
  await teamChip.click();

  await expect(page.getByTestId("member-table")).toBeVisible();
  // At least one member (in dev-allow, the `local-dev` principal is admitted).
  await expect(page.locator('[data-testid^="member-row-"]').first()).toBeVisible();
  // Exactly one member carries the Delegate role (role variety).
  await expect(page.locator(".role-badge--delegate")).toHaveCount(1);

  // The sharing inspector auto-selects the first asset (echo) and shows its grants,
  // including the demo TEAM grant (root, active) — the resolve path's source.
  await expect(page.getByTestId("grant-inspector")).toBeVisible();
  await expect(page.getByTestId("grant-asset-pick-kx/recipes/echo")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByTestId("grant-table")).toBeVisible();
  const teamGrant = page.getByTestId("grant-row-kx/teams/workspace");
  await expect(teamGrant).toBeVisible();
  await expect(teamGrant).toContainText("Root");

  // With an asset auto-selected, each member resolves a warrant on it (membership ∩
  // grant) — the kx-fleet composition, surfaced in the member table.
  await expect(page.locator('[data-testid^="member-warrant-"]').first()).toBeVisible({
    timeout: 30_000,
  });

  // The pickers are CHIP buttons (aria-pressed), never a controlled <select>.
  await expect(page.locator('[data-testid="systems-section"] select')).toHaveCount(0);
});

test("Systems: switching the inspected asset updates the grants + the resolved warrants", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-systems").click();
  await expect(page.getByTestId("grant-asset-pick-kx/recipes/echo")).toBeVisible({
    timeout: 30_000,
  });

  // Switch to the fanout recipe (no team grant → its grant table differs from echo's).
  const fanoutChip = page.getByTestId("grant-asset-pick-kx/recipes/passthrough-dag");
  await expect(fanoutChip).toBeVisible();
  await fanoutChip.click();
  await expect(fanoutChip).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("grant-table")).toBeVisible();
  // Echo's team grant is NOT on fanout (the team was granted only on echo).
  await expect(page.getByTestId("grant-row-kx/teams/workspace")).toHaveCount(0);
});
