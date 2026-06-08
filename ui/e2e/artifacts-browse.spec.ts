import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("artifacts: browse a run's committed outputs and review one", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Run the 5-node fanout recipe so the run has several committed artifacts.
  await runRecipe(page, { handle: "kx/recipes/fanout-demo" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBe(5);

  // In-app navigate to Artifacts (a full page.goto would drop the in-memory connection).
  await page.getByTestId("nav-artifacts").click();
  await expect(page.getByTestId("artifacts-section")).toBeVisible();
  // The just-run instance is selectable; its gallery lists the committed outputs.
  await expect(page.getByTestId("artifact-run-pick")).toBeVisible();
  await expect(page.getByTestId("artifact-gallery")).toBeVisible({ timeout: 30_000 });
  const rows = page.locator(".artifact-list__row");
  await expect.poll(() => rows.count(), { timeout: 30_000 }).toBeGreaterThan(0);

  // Expand one artifact → its decoded content renders (fail-closed ArtifactView).
  await rows.first().click();
  await expect(page.getByTestId("artifact-view")).toBeVisible({ timeout: 30_000 });
});
