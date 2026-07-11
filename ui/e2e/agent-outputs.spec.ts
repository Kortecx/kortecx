import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("Data Lab Agent Outputs: a committed run's outputs are reviewable in the multi-modal viewer", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Produce committed agent outputs: the 5-node fanout recipe commits several Motes,
  // each of which folds into the Morphic capture action exhaust (ListCaptureRecords).
  await runRecipe(page, { handle: "kx/recipes/passthrough-dag" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 60_000,
    })
    .toBeGreaterThan(0);

  await gotoViaPalette(page, "datasets");
  await expect(page.getByTestId("datasets-section")).toBeVisible();
  await expect(page.getByTestId("agent-outputs")).toBeVisible();

  // The capture fold is a fast background poller; re-navigate (Chat ↔ Data Lab)
  // inside the poll to force a fresh fetch until the committed outputs appear (no
  // page reload — the console token is held in memory only).
  const rows = page.locator('[data-testid^="agent-output-"]');
  await expect
    .poll(
      async () => {
        await page.getByTestId("nav-chat").click();
        await gotoViaPalette(page, "datasets");
        return rows.count();
      },
      { timeout: 30_000 },
    )
    .toBeGreaterThan(0);

  // Open one output → the multi-modal AssetViewer renders its committed content from
  // the content-addressed store (a blob URL / Monaco — never a remote src).
  await rows.first().click();
  await expect(page.getByTestId("asset-viewer")).toBeVisible({ timeout: 30_000 });

  // The "ReAct turns" filter narrows to records that carry a settled branch; a PURE
  // fanout run has none, so the lens shows its honest empty state (scoped to the
  // Agent Outputs panel — the Datasets panel renders its own empty state too).
  await page.getByTestId("agent-outputs-filter-react").click();
  await expect(page.getByTestId("agent-outputs").getByTestId("empty-state")).toBeVisible();
});
