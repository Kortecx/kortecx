import { expect, test } from "@playwright/test";
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

  // Lands on /runs (the echo handle is prefilled)
  await expect(page.getByRole("heading", { name: /run a recipe/i })).toBeVisible();
  await page.getByRole("button", { name: /submit run/i }).click();

  // Run-detail: the DAG table appears and a Mote pill flips to COMMITTED via the poll loop.
  await expect(page.getByTestId("mote-table")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).first()).toBeVisible(
    { timeout: 30_000 },
  );
});
