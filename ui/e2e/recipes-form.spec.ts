import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("blueprint catalog: pick echo, fill its generated form, run → COMMITTED", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-recipes").click();
  // The catalog (ListRecipes) + the generated form (GetRecipeForm) render — the
  // `topic` field is server-described, not hardcoded.
  await expect(page.getByTestId("recipe-catalog")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("recipe-pick-kx/recipes/echo").click();
  await expect(page.getByTestId("field-topic")).toBeVisible();

  const topic = page.getByTestId("field-topic");
  await topic.click();
  await topic.pressSequentially("incident review");
  await page.getByRole("button", { name: /run blueprint/i }).click();

  // Routes to the live run-detail DAG; the run commits over the real gRPC-web path.
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).first()).toBeVisible(
    { timeout: 30_000 },
  );
});

test("blueprint form validation: a required field blocks submit until filled", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-recipes").click();
  await expect(page.getByTestId("recipe-catalog")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("recipe-pick-kx/recipes/echo").click();
  await expect(page.getByTestId("field-topic")).toBeVisible();

  // Submit with the required `topic` blank → an inline validation error, no run.
  await page.getByRole("button", { name: /run blueprint/i }).click();
  await expect(page.getByRole("alert")).toContainText(/required/i);
  await expect(page.getByTestId("mote-dag")).toHaveCount(0);

  // Fill it and the run proceeds.
  const topic = page.getByTestId("field-topic");
  await topic.click();
  await topic.pressSequentially("ok now");
  await page.getByRole("button", { name: /run blueprint/i }).click();
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
});
