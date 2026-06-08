import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
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

  // Run the model-free multi-node fanout recipe (no free-params) via its catalog form.
  await runRecipe(page, { handle: "kx/recipes/fanout-demo" });

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
