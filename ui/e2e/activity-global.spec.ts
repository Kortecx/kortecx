import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("activity drawer: the global landing feed, quick actions, and run drill-in", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // Seed a run so the global feed has rows to land on.
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "drawer" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // The drawer's landing (no run selected) is the GLOBAL feed + quick actions.
  await page.getByTestId("activity-toggle").click();
  await expect(page.getByTestId("activity-drawer")).toBeVisible();
  await expect(page.getByTestId("quick-actions")).toBeVisible();
  await expect(page.getByTestId("global-event-row").first()).toBeVisible({ timeout: 20_000 });

  // A quick action navigates + closes the drawer.
  await page.getByTestId("quick-new-chat").click();
  await expect(page.getByTestId("activity-drawer")).not.toBeVisible();
  await expect(page.getByTestId("chat-panel")).toBeVisible({ timeout: 15_000 });

  // Reopen: a feed row's run chip drills into the run-scoped panel (metrics +
  // per-run feed). The selection then persists for the drawer session.
  await page.getByTestId("activity-toggle").click();
  await expect(page.getByTestId("global-event-row").first()).toBeVisible({ timeout: 20_000 });
  await page.getByTestId("global-event-run").first().click();
  await expect(page.getByTestId("metrics-panel")).toBeVisible({ timeout: 15_000 });
});
