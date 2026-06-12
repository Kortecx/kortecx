import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("artifacts: a committed output renders in the offline Monaco code viewer", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await runRecipe(page, { handle: "kx/recipes/fanout-demo" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBe(5);

  // PR-2 merge (D141.1): Artifacts is a TAB of the run's detail page.
  await page.getByTestId("run-tab-artifacts").click();
  await expect(page.getByTestId("artifacts-tab")).toBeVisible();
  await expect(page.getByTestId("artifact-gallery")).toBeVisible({ timeout: 30_000 });
  const rows = page.locator(".artifact-list__row");
  await expect.poll(() => rows.count(), { timeout: 30_000 }).toBeGreaterThan(0);

  await rows.first().click();
  await expect(page.getByTestId("artifact-view")).toBeVisible({ timeout: 30_000 });
  // The read-only viewer is real Monaco (loaded from the SAME-ORIGIN bundle — no CDN,
  // proving the offline self-hosting), carrying the decoded payload.
  const viewer = page.getByTestId("artifact-view-body");
  await expect(viewer).toBeVisible({ timeout: 30_000 });
  await expect(viewer.locator(".monaco-editor")).toBeVisible({ timeout: 30_000 });
});
