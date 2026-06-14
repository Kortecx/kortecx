/**
 * PR-A: the /workflows page is a `Workflows | Runs` toggle — the workflow
 * DEFINITIONS table and the run-history table both live here. Clicking a
 * workflow row opens its definition popup (contract + view + the new-window
 * button), per the user's point-4 spec.
 */

import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("workflows page: the Workflows|Runs toggle + the workflow-definition popup", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("runs-section")).toBeVisible();

  // Both tabs are present; default lands on Runs.
  await expect(page.getByTestId("workflows-tab-workflows")).toBeVisible();
  await expect(page.getByTestId("workflows-tab-runs")).toHaveAttribute("aria-pressed", "true");

  // Switch to the Workflows (definitions) table — the catalog renders as ROWS.
  await page.getByTestId("workflows-tab-workflows").click();
  await expect(page.getByTestId("workflows-list")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("workflows-list")).toContainText("kx/recipes/echo");

  // Click a workflow row → the definition popup (point 4): contract + view +
  // the new-window button (which lives ONLY in the popup).
  await page.getByTestId("workflow-open-kx/recipes/echo").click();
  const drawer = page.getByTestId("workflow-detail-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer.getByTestId("workflow-definition")).toContainText("kx/recipes/echo", {
    timeout: 30_000,
  });
  const newtab = drawer.getByTestId("workflow-open-newtab");
  await expect(newtab).toHaveAttribute("target", "_blank");
  await expect(newtab).toHaveAttribute("rel", "noopener noreferrer");

  // The popup's Run links to the Blueprints run form for that workflow.
  await drawer.getByTestId("workflow-run").click();
  await expect(page).toHaveURL(/\/recipes\?handle=/);
});
