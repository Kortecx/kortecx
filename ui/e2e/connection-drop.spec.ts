import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
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
  await connectConsole(page, gw);

  await runRecipe(page);
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // The gateway disappears.
  gw.stop();
  gw = undefined;

  // A manual refresh now hits an unreachable gateway → a clear error notice with a Retry
  // affordance (the durability property: the UI degrades gracefully, it does not hang).
  // Use the run-detail "Refresh" (exact) — the navbar has a separate "Refresh all data".
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByTestId("error-notice")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("button", { name: /retry/i })).toBeVisible();
});
