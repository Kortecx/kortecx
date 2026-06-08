import { type Page, expect } from "@playwright/test";
import type { Gateway } from "./serve";

/**
 * Drive the connect screen to a connected console. Optionally set the advanced WS
 * endpoint (for the Activity live tail). Resolves once the Activity dashboard shows.
 */
export async function connectConsole(
  page: Page,
  gw: Gateway,
  opts: { wsEndpoint?: string } = {},
): Promise<void> {
  await page.goto("/connect");
  await page.getByLabel(/gateway endpoint/i).fill(gw.endpoint);
  if (opts.wsEndpoint) {
    await page.getByText(/^advanced$/i).click();
    await page.getByLabel(/ws bridge endpoint/i).fill(opts.wsEndpoint);
  }
  await page.getByRole("button", { name: /^connect$/i }).click();
  await expect(page.getByTestId("activity-panel")).toBeVisible({ timeout: 30_000 });
}

/**
 * Submit a recipe via the UI-2 recipe catalog: navigate to Recipes, select the
 * handle, fill any free-param fields, and click "Run recipe". Controlled inputs
 * are filled with click + pressSequentially (a bulk fill() can leave React state
 * stale — the recorded e2e gotcha). Defaults to the echo recipe with a topic.
 */
export async function runRecipe(
  page: Page,
  opts: { handle?: string; fields?: Record<string, string> } = {},
): Promise<void> {
  const handle = opts.handle ?? "kx/recipes/echo";
  const fields = opts.fields ?? (handle === "kx/recipes/echo" ? { topic: "hello" } : {});
  await page.getByTestId("nav-recipes").click();
  await expect(page.getByTestId("recipe-catalog")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId(`recipe-pick-${handle}`).click();
  // Wait for the form to reflect the SELECTED recipe before interacting — switching
  // handles re-fetches GetRecipeForm, and clicking too early would submit the old
  // recipe's (still-visible) form.
  await expect(page.locator(`[data-testid="recipe-form"][data-recipe="${handle}"]`)).toBeVisible({
    timeout: 30_000,
  });
  for (const [name, value] of Object.entries(fields)) {
    const input = page.getByTestId(`field-${name}`);
    await input.click();
    await input.pressSequentially(value);
  }
  await page.getByRole("button", { name: /run recipe/i }).click();
}
