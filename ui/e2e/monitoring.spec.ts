import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("monitoring: the gateway-wide dashboard renders real telemetry", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Run something first so there is at least one run to roll up.
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "monitor" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("nav-monitor").click();
  await expect(page.getByTestId("monitoring-section")).toBeVisible({ timeout: 15_000 });

  // Real numbers, not placeholders: the hero metric grid + a live gateway-health pill.
  await expect(page.getByText("Runs", { exact: true }).first()).toBeVisible();
  await expect(page.getByTestId("health-indicator")).toHaveText("LIVE");
  // The Runs panel rolls up the just-run blueprint handle (real local+durable history).
  await expect(page.getByText("kx/recipes/echo").first()).toBeVisible({ timeout: 15_000 });
  // The self-correction / ReAct / capture panels render (data OR an honest empty/degrade
  // note) — never a crash. At least one tally/empty note is present.
  await expect(page.getByRole("heading", { name: "Self-correction" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "ReAct turns" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Action capture" })).toBeVisible();

  // RC6a: the Gateway-health card folds MCP connector health (read-only + Test). With
  // no connectors registered it renders the honest empty/not-wired state (never a
  // crash), and the manage affordance links back to Integrations (where revoke lives).
  await expect(page.getByRole("heading", { name: "Connectors" })).toBeVisible();
  await expect(page.getByTestId("monitor-connections-manage")).toBeVisible();
});
