import { expect, test } from "@playwright/test";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("a gateway drop mid-session surfaces a clear, retryable error (no hang/crash)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  await page.goto("/connect");
  const endpoint = page.getByLabel(/gateway endpoint/i);
  await endpoint.fill(gw.endpoint);
  await expect(endpoint).toHaveValue(gw.endpoint);
  await page.getByRole("button", { name: /^connect$/i }).click();
  await page.getByRole("button", { name: /submit run/i }).click();
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // The gateway disappears.
  gw.stop();
  gw = undefined;

  // A manual refresh now hits an unreachable gateway → a clear error notice with a Retry
  // affordance (the durability property: the UI degrades gracefully, it does not hang).
  await page.getByRole("button", { name: /refresh/i }).click();
  await expect(page.getByTestId("error-notice")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("button", { name: /retry/i })).toBeVisible();
});
