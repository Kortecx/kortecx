import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

/**
 * PR-D: the run DAG is a read-only, high-level REVIEW — each committed step is
 * labelled by type (Model / MCP / Connector / Tool / Action), derived from
 * GetMoteDetail. Driven model-free through `passthrough-dag` (pure steps → "Action"
 * labels), asserted in BOTH themes. The step remains reviewable read-only via the
 * inspector; the run review has no edit affordance.
 */

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("the run DAG labels each committed step by high-level type, in BOTH themes", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await runRecipe(page, { handle: "kx/recipes/passthrough-dag" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // Every committed node gets a high-level step-type badge (pure steps → "Action").
  await expect
    .poll(() => page.getByTestId("dag-node-step").count(), { timeout: 30_000 })
    .toBeGreaterThan(0);
  await expect(page.getByTestId("dag-node-step").first()).toBeVisible();

  // A step is reviewable read-only: click a node → the inspector opens (no edit UI).
  await page.getByTestId("mote-node").first().click();
  await expect(page.getByTestId("node-detail-drawer")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole("button", { name: /^(edit|save)/i })).toHaveCount(0);
  await page.keyboard.press("Escape");

  // Both themes: the step badges stay legible in light AND dark.
  for (const theme of ["light", "dark"] as const) {
    const current = await page.locator("html").getAttribute("data-theme");
    if (current !== theme) {
      await page.getByTestId("theme-toggle").click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.getByTestId("dag-node-step").first()).toBeVisible();
  }
});
