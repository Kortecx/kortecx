import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("dag: clicking a Mote opens the detail drawer with its committed result", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await runRecipe(page, { handle: "kx/recipes/passthrough-dag" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBe(5);

  const nodes = page.getByTestId("mote-node");
  const clickTarget = nodes.first();
  // The clicked node's reactflow translate transform — its LAYOUT position within the
  // pane. This is immune to the entrance fitView's viewport pan AND to reactflow's
  // on-click DOM reorder (read by the node's own element); it changes ONLY on a relayout.
  const nodeTransform = (loc: typeof clickTarget) =>
    loc.evaluate((el) => {
      const node = el.closest(".react-flow__node");
      return node ? getComputedStyle(node).transform : "";
    });
  const layoutBefore = await nodeTransform(clickTarget);

  await clickTarget.click();
  const drawer = page.getByTestId("node-detail-drawer");
  await expect(drawer).toBeVisible({ timeout: 15_000 });
  // It shows the Mote's state + its committed result in the read-only Monaco viewer.
  await expect(drawer.getByTestId("state-pill")).toBeVisible();
  await expect(page.getByTestId("node-detail-result")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("node-detail-result").locator(".monaco-editor")).toBeVisible({
    timeout: 30_000,
  });

  // The graph did NOT relayout: the node's layout transform is byte-identical (a relayout
  // would change its translate; a benign viewport pan/scrollbar leaves it untouched).
  expect(await nodeTransform(clickTarget)).toBe(layoutBefore);

  // Escape closes the drawer.
  await page.keyboard.press("Escape");
  await expect(drawer).toBeHidden({ timeout: 15_000 });
});
