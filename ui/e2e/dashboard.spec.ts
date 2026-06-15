import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("dashboard: honest-empty before any run, real KPIs + recent runs after (both themes)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // The Dashboard is a NEW Workspace nav item; chat is STILL the default (D137).
  await page.getByTestId("nav-dashboard").click();
  await expect(page.getByTestId("dashboard-section")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("dashboard-kpis")).toBeVisible();
  // Honest-empty recent runs before anything has executed.
  await expect(page.getByText(/No runs yet/i)).toBeVisible();
  // GR15: none of the reference app's FABRICATED cards leak in.
  await expect(page.getByText(/active agents/i)).toHaveCount(0);
  await expect(page.getByText(/success rate/i)).toHaveCount(0);
  await expect(page.getByText(/tasks today/i)).toHaveCount(0);

  // Seed a real run, then the dashboard reflects it (local + durable history).
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "dash" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("nav-dashboard").click();
  await expect(page.getByTestId("dashboard-run").first()).toBeVisible({ timeout: 15_000 });

  // BOTH THEMES (D142.1 / GR13): the landing renders under the dark palette.
  await page.getByTestId("theme-toggle").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.getByTestId("dashboard-kpis")).toBeVisible();
  await expect(page.getByTestId("dashboard-run").first()).toBeVisible();
});
