import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("submit fanout-demo → the live DAG renders the whole fan-out → gather graph (COMMITTED)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-recipes").click();
  await expect(page.getByRole("heading", { name: "Recipes", exact: true })).toBeVisible();
  // Focus then type the handle/args (pressSequentially fires per-keystroke input
  // events the React controlled inputs catch; a bulk fill() can leave state stale).
  const handle = page.getByLabel(/recipe handle/i);
  await handle.click();
  await handle.fill("");
  await handle.pressSequentially("kx/recipes/fanout-demo");
  const args = page.getByLabel(/args \(json/i);
  await args.click();
  await args.fill("");
  await args.pressSequentially("{}");
  await page.getByRole("button", { name: /submit run/i }).click();

  // The live DAG materializes the entire graph: root + 3 children + gather = 5 nodes.
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.getByTestId("mote-node").count(), { timeout: 30_000 }).toBe(5);

  // Every Mote reaches COMMITTED (the gather joins all three children) — the headline
  // "watch the DAG execute" beat, driven model-free through a real gRPC-web gateway.
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBe(5);
});
