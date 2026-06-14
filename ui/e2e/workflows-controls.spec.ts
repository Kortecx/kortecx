/**
 * PR-2.1 review-feedback round: the Workflows page controls (New workflow ·
 * Clone-prefill · Rename · the honest "Clear local history"), the Blueprints
 * catalog TABLE + the Monaco contract popup, the printable demo result, and
 * the agent toggle's honest ABSENCE on an FFI-free serve.
 */

import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("workflows: clone prefills the blueprint form; rename + clear-local-history stay honest", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Run echo with a distinctive topic, then land on Workflows.
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "clone-me" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("run-list")).toBeVisible();

  // The row is LABELED by its recipe handle (session record).
  const row = page.locator(".run-list__item").first();
  await expect(row).toContainText("kx/recipes/echo");

  // Rename (client-local) → the row shows the custom name.
  await row.getByTestId("run-rename").click();
  await row.getByTestId("run-rename-input").fill("incident triage");
  await row.getByTestId("run-rename-input").press("Enter");
  await expect(row).toContainText("incident triage");

  // Clone → the Blueprints form opens PRE-FILLED with the prior args.
  await row.getByTestId("run-clone").click();
  await expect(page.getByTestId("recipe-table")).toBeVisible({ timeout: 30_000 });
  await expect(
    page.locator('[data-testid="recipe-form"][data-recipe="kx/recipes/echo"]'),
  ).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("field-topic")).toHaveValue("clone-me");

  // Back on Workflows: clearing LOCAL history keeps the durable journal row
  // (truth stays) — and the client-local RENAME survives too (separate store).
  await page.getByTestId("nav-runs").click();
  await page.getByTestId("clear-local-history").click();
  await expect(page.getByTestId("run-list")).toBeVisible({ timeout: 30_000 });
  const durable = page.locator(".run-list__item").first();
  await expect(durable).toContainText("journal"); // the recovered-from-journal badge
  await expect(durable).toContainText("incident triage"); // renames outlive the clear

  // Dropping the rename falls back to the fingerprint→handle JOIN (PR-2.1):
  // the durable row is still NAMED by its recipe, not a bare hex id.
  await durable.getByTestId("run-rename").click();
  await durable.getByTestId("run-rename-input").fill("");
  await durable.getByTestId("run-rename-input").press("Enter");
  await expect(durable).toContainText("kx/recipes/echo");

  // "New workflow" links to the Blueprints authoring home (D141.1).
  await page.getByTestId("new-workflow").click();
  await expect(page).toHaveURL(/\/recipes$/);
});

test("blueprints: the catalog is a table; View opens the contract in Monaco", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-recipes").click();
  await expect(page.getByTestId("recipe-table")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("recipe-view-kx/recipes/echo").click();
  const viewer = page.getByTestId("blueprint-viewer");
  await expect(viewer).toBeVisible();
  await expect(viewer.getByTestId("blueprint-contract")).toContainText("kx/recipes/echo", {
    timeout: 30_000,
  });
  // The contract renders in the offline read-only Monaco (D141.2).
  await expect(viewer.locator(".monaco-editor")).toBeVisible({ timeout: 30_000 });
  await page.keyboard.press("Escape");
  await expect(viewer).toBeHidden();
});

test("echo results render as readable TEXT (honest passthrough) and the agent toggle is honestly absent FFI-free", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // The FFI-free serve provisions no react recipe ⇒ NO agent toggle (don't-fake-gaps).
  await expect(page.getByTestId("chat-panel")).toBeVisible();
  await expect(page.getByTestId("chat-mode")).toHaveCount(0);

  // A committed echo result is readable text in the inspector's Result pane
  // (GR15: `echo` commits its bound `topic` verbatim, never a placeholder).
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "readable" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBeGreaterThan(0);
  await page.getByTestId("mote-node").first().click();
  await expect(page.getByTestId("node-detail-drawer")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("node-detail-result")).toContainText("readable", {
    timeout: 30_000,
  });
});
