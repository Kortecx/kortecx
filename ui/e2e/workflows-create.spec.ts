/**
 * The durable-Workflow create journey — its own `/workflows/create` page:
 * name the workflow over the embedded builder canvas, Save (`SaveWorkflow` hits
 * the real gateway — no model needed: a served-model agent step is valid by the
 * portable convention), land on the definition page, find it in the catalog's
 * "Your workflows" group, and open the definition-history drawer (the FIRST
 * history-drawer e2e — the unit suites cover its logic, nothing covered its
 * portal/overlay geometry against a real serve until now).
 *
 * No scaffold machinery exists on this journey by design: the save IS the
 * authoring act, so the terminal state is simply the definition page.
 */

import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("new workflow: author → save → catalog lists it → def page → history drawer", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // The Workflows home: "New workflow" NAVIGATES to the dedicated create page.
  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("runs-section")).toBeVisible();
  await page.getByTestId("workflows-new").click();
  await expect(page).toHaveURL(/\/workflows\/create/);
  await expect(page.getByTestId("workflow-create-form")).toBeVisible();

  // The embedded builder canvas is the structure surface (it starts with one
  // served-model agent step — valid as-is: the server binds the model at run).
  await expect(page.getByTestId("builder-canvas")).toBeVisible({ timeout: 30_000 });

  // Name it; the handle follows the name (workflows/local/<sanitized>).
  const handle = "workflows/local/morning-digest";
  await page.getByTestId("workflow-name").fill("Morning Digest");
  await expect(page.getByTestId("workflow-handle")).toHaveValue(handle);
  await page.getByTestId("workflow-description").fill("Summarize the overnight items");

  // Save is the WHOLE authoring act — it lands on the definition page.
  await expect(page.getByTestId("workflow-save")).toBeEnabled();
  await page.getByTestId("workflow-save").click();
  await expect(page).toHaveURL(/\/workflows\/def\//, { timeout: 15_000 });
  await expect(page.getByTestId("workflow-def")).toBeVisible();
  await expect(page.getByTestId("workflow-def-name")).toContainText("Morning Digest");
  await expect(page.getByTestId("workflow-def-description")).toContainText(
    "Summarize the overnight items",
  );
  await expect(page.getByTestId("workflow-def-steps")).toBeVisible();
  await expect(page.getByTestId("workflow-def-step-1")).toBeVisible();
  // Not a draft ⇒ Run is offered (Finish draft is the draft-only swap).
  await expect(page.getByTestId("workflow-def-run")).toBeVisible();
  await expect(page.getByTestId("workflow-def-finish")).toHaveCount(0);

  // Back on the catalog, the saved workflow leads the page under "Your workflows".
  await page.getByTestId("nav-runs").click();
  const card = page.getByTestId(`workflow-def-card-${handle}`);
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card).toContainText("Morning Digest");
  await expect(card).toContainText("1 step");

  // The card opens the definition page.
  await card.getByTestId(`workflow-def-open-${handle}`).click();
  await expect(page).toHaveURL(/\/workflows\/def\//);
  await expect(page.getByTestId("workflow-def-name")).toContainText("Morning Digest");

  // The definition-history drawer: the save was recorded as version 1 — shown
  // newest-first with the newest row pinned "current" (not restorable).
  await page.getByTestId("workflow-def-history").click();
  const drawer = page.getByTestId("workflow-history");
  await expect(drawer).toBeVisible();
  await expect(page.getByTestId("workflow-history-list")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("workflow-history-current")).toBeVisible();
  // Ungated: no blocked notice (a workflow restore has no lock/scaffold gate).
  await expect(page.getByTestId("workflow-history-blocked")).toHaveCount(0);
  // Escape closes the drawer (no confirm is open).
  await page.keyboard.press("Escape");
  await expect(drawer).toBeHidden();
});
