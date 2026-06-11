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

  await runRecipe(page, { handle: "kx/recipes/fanout-demo" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBe(5);

  const nodes = page.getByTestId("mote-node");
  const first = nodes.first();
  // Capture a node bounding box before opening the drawer (no-thrash assertion).
  const before = await first.boundingBox();

  await first.click();
  const drawer = page.getByTestId("node-detail-drawer");
  await expect(drawer).toBeVisible({ timeout: 15_000 });
  // It shows the Mote's state + its committed result in the read-only Monaco viewer.
  await expect(drawer.getByTestId("state-pill")).toBeVisible();
  await expect(page.getByTestId("node-detail-result")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("node-detail-result").locator(".monaco-editor")).toBeVisible({
    timeout: 30_000,
  });

  // The graph did NOT relayout: the same node sits at the same position.
  const after = await first.boundingBox();
  expect(Math.abs((after?.x ?? 0) - (before?.x ?? 0))).toBeLessThan(2);
  expect(Math.abs((after?.y ?? 0) - (before?.y ?? 0))).toBeLessThan(2);

  // Escape closes the drawer.
  await page.keyboard.press("Escape");
  await expect(drawer).toBeHidden({ timeout: 15_000 });
});
