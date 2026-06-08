import { expect, test } from "@playwright/test";
import { runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("connect → submit echo → watch a Mote reach COMMITTED (real gRPC-web + CORS)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  await page.goto("/connect");
  await expect(page.getByRole("heading", { name: /connect to a gateway/i })).toBeVisible();

  const endpoint = page.getByLabel(/gateway endpoint/i);
  await endpoint.fill(gw.endpoint);
  await expect(endpoint).toHaveValue(gw.endpoint);
  await page.getByRole("button", { name: /^connect$/i }).click();

  // Lands on the Activity dashboard; go to Recipes and run echo via its generated form.
  await expect(page.getByTestId("activity-panel")).toBeVisible({ timeout: 30_000 });
  await runRecipe(page, { fields: { topic: "incidents" } });

  // Run-detail defaults to the live DAG: a Mote node appears and flips to COMMITTED
  // via the projection poll loop (the real reactflow canvas + gRPC-web path).
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("mote-node").first()).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).first()).toBeVisible(
    { timeout: 30_000 },
  );

  // The table toggle still works (the proven scale + a11y surface stays reachable).
  await page.getByRole("button", { name: /^table$/i }).click();
  await expect(page.getByTestId("mote-table")).toBeVisible();
});
