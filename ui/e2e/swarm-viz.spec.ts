import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

/**
 * PR-B: a swarm run (parallel branches → gather) is legible over the shipped DAG —
 * a fan-in roll-up strip + a marked gather node, all inferred from the projection
 * (no new RPC). Driven model-free through the `passthrough-dag` recipe (root → 3
 * branches → gather), and asserted in BOTH themes.
 */

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("a fan-out → gather run shows the swarm overview + a marked gather, in BOTH themes", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // The model-free multi-node fanout recipe: root → 3 branches → gather.
  await runRecipe(page, { handle: "kx/recipes/passthrough-dag" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // The swarm roll-up appears with one row per branch (the 3 children fan into the gather).
  await expect(page.getByTestId("swarm-overview")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("swarm-pattern-badge")).toBeVisible();
  await expect
    .poll(() => page.getByTestId("swarm-branch-row").count(), { timeout: 30_000 })
    .toBe(3);

  // The gather (fan-in sink) node is marked on the canvas (exactly one).
  await expect(page.locator('[data-swarm-role="gather"]')).toHaveCount(1);

  // Both themes: the overview stays visible + legible in light AND dark.
  for (const theme of ["light", "dark"] as const) {
    const current = await page.locator("html").getAttribute("data-theme");
    if (current !== theme) {
      await page.getByTestId("theme-toggle").click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.getByTestId("swarm-overview")).toBeVisible();
    await expect(page.getByTestId("swarm-pattern-badge")).toBeVisible();
  }
});
