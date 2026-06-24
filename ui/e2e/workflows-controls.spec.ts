/**
 * PR-4.1b card overhaul: the Workflows page CARDS + per-card action menu (New
 * workflow · Clone-prefill · Rename · the honest "Clear local history"), the
 * Blueprints catalog CARD GRID + the Monaco contract popup, the readable
 * passthrough result, and the agent toggle's honest ABSENCE on an FFI-free serve.
 */

import { expect, test } from "@playwright/test";
import { connectConsole, gotoRunHistory, gotoViaPalette, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("workflows: a run row's detail popup clones (prefilled), renames, and clear-local-history stays honest", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Run echo with a distinctive topic, then land on Workflows → Runs.
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "clone-me" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await gotoRunHistory(page);
  await expect(page.getByTestId("run-list")).toBeVisible();

  // The row carries its recipe handle (clean name = "Echo").
  const row = page.getByTestId("run-row").first();
  await expect(row).toContainText("kx/recipes/echo");

  // Open the detail popup; rename (client-local) → the row shows the custom name.
  await row.getByTestId("run-open").click();
  const drawer = page.getByTestId("run-detail-drawer");
  await expect(drawer).toBeVisible();
  await drawer.getByTestId("run-rename-input").fill("incident triage");
  await drawer.getByTestId("run-rename").click();
  await page.keyboard.press("Escape");
  await expect(row).toContainText("incident triage");

  // Clone (in the popup) → the Blueprints run form opens PRE-FILLED with the args.
  await row.getByTestId("run-open").click();
  await drawer.getByTestId("run-clone").click();
  await expect(page.getByTestId("recipe-catalog")).toBeVisible({ timeout: 30_000 });
  await expect(
    page.locator('[data-testid="recipe-form"][data-recipe="kx/recipes/echo"]'),
  ).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("field-topic")).toHaveValue("clone-me");

  // Back on Workflows → Runs: clearing LOCAL history keeps the durable journal row
  // (truth stays) — and the client-local RENAME survives too (separate store).
  await gotoRunHistory(page);
  await page.getByTestId("clear-local-history").click();
  await expect(page.getByTestId("run-list")).toBeVisible({ timeout: 30_000 });
  const durable = page.getByTestId("run-row").first();
  await expect(durable).toContainText("journal"); // the recovered-from-journal badge
  await expect(durable).toContainText("incident triage"); // renames outlive the clear

  // Dropping the rename falls back to the humanized handle + its raw-handle chip.
  await durable.getByTestId("run-open").click();
  await drawer.getByTestId("run-rename-input").fill("");
  await drawer.getByTestId("run-rename").click();
  await page.keyboard.press("Escape");
  await expect(durable).toContainText("kx/recipes/echo");
});

test("blueprints: the catalog is a card grid; the menu's View opens the contract in Monaco", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await gotoViaPalette(page, "recipes");
  await expect(page.getByTestId("recipe-catalog")).toBeVisible({ timeout: 30_000 });

  // Open the echo card's action menu → View contract.
  const bp = page
    .getByTestId("blueprint-card")
    .filter({ has: page.getByTestId("recipe-pick-kx/recipes/echo") });
  await bp.getByTestId("blueprint-menu").click();
  await bp.getByTestId("recipe-view-kx/recipes/echo").click();
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
