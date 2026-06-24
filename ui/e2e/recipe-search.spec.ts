import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("blueprint search (SearchRecipes): an intent surfaces ranked recipes", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await gotoViaPalette(page, "recipes");
  await expect(page.getByTestId("blueprint-search")).toBeVisible({ timeout: 30_000 });

  // Search by tag/intent ⇒ ranked hits appear (display-only — never invokes).
  await page.getByTestId("blueprint-search-input").fill("passthrough");
  await expect(page.getByTestId("blueprint-search-results")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("blueprint-search-hit").first()).toBeVisible();

  // A clearly-unrelated query surfaces no matches (honest empty, not a fake).
  // POC-5c CI-hardening (GR23): the prior probe "zzz-no-such-recipe-xyz" shared the
  // token "recipe" with the recipe handles, so the DiscoveryIndex's cosine-ANN
  // occasionally ranked it just inside the relevance boundary and returned a stray
  // hit — flaking this exact-empty assertion (reproduced in CI on Linux). A query with
  // no shared tokens or concepts stays well clear of the boundary, so the honest-empty
  // state is reliable. (The positive "passthrough" hit above is unaffected.)
  await page.getByTestId("blueprint-search-input").fill("platypus saxophone umbrella");
  await expect(page.getByTestId("blueprint-search-results")).toHaveCount(0, { timeout: 30_000 });
});
