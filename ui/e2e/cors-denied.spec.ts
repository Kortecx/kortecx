import { expect, test } from "@playwright/test";
import { type Gateway, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("a deny-by-default gateway (origin not allowed) fails the connect probe cleanly", async ({
  page,
}) => {
  // No --cors-origin → the SPA's browser origin is NOT allowed → the gRPC-web call is blocked.
  gw = await spawnGateway();

  await page.goto("/connect");
  const endpoint = page.getByLabel(/gateway endpoint/i);
  await endpoint.fill(gw.endpoint);
  await expect(endpoint).toHaveValue(gw.endpoint);
  await page.getByRole("button", { name: /^connect$/i }).click();

  // A clear error notice appears and we stay on the connect screen (no hang, no crash).
  await expect(page.getByTestId("error-notice")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole("heading", { name: /connect to a gateway/i })).toBeVisible();
  await expect(page.getByTestId("conn-status")).toHaveAttribute("data-status", "disconnected");
});
