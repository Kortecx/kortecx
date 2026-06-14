import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("blueprint search (SearchRecipes): an intent surfaces ranked recipes", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-recipes").click();
  await expect(page.getByTestId("blueprint-search")).toBeVisible({ timeout: 30_000 });

  // Search by tag/intent ⇒ ranked hits appear (display-only — never invokes).
  await page.getByTestId("blueprint-search-input").fill("passthrough");
  await expect(page.getByTestId("blueprint-search-results")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("blueprint-search-hit").first()).toBeVisible();

  // A nonsense query surfaces no matches (honest empty, not a fake).
  await page.getByTestId("blueprint-search-input").fill("zzz-no-such-recipe-xyz");
  await expect(page.getByTestId("blueprint-search-results")).toHaveCount(0, { timeout: 30_000 });
});
