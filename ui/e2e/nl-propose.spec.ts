import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { expectOverlayAboveNavbar } from "./fixtures/overlay";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

// C4 (D209.3): the NL "Describe a workflow" propose→confirm affordance is wired into the
// builder. This model-free e2e proves the UI wiring — the panel opens as a portaled overlay
// that clears the navbar, takes a goal, and dismisses. The full propose→run path (goal → a
// multi-step DAG that executes) is proven on a live model by the Rust in-process witness
// (`app_live_serve.rs::propose_workflow_authors_a_multistep_dag_and_runs_live`).
test("builder: the Describe-a-workflow panel opens, clears the navbar, and closes (C4)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await gotoViaPalette(page, "recipes");
  await page.getByTestId("new-blueprint").click();
  await expect(page.getByTestId("builder-canvas")).toBeVisible({ timeout: 30_000 });

  // Open the NL propose panel from the builder toolbar.
  await page.getByTestId("builder-propose").click();
  await expect(page.getByTestId("builder-propose-panel")).toBeVisible();
  // It is a portaled overlay that paints OVER the sticky navbar (the section-drawer pattern).
  await expectOverlayAboveNavbar(page, "builder-propose-panel");

  // The goal input accepts text; "Propose a plan" enables once a goal is present.
  await expect(page.getByTestId("builder-propose-submit")).toBeDisabled();
  await page.getByTestId("builder-propose-goal").fill("Compare two durable-execution engines.");
  await expect(page.getByTestId("builder-propose-submit")).toBeEnabled();

  // Dismissible with Escape (like the other overlays).
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("builder-propose-panel")).toHaveCount(0);
});
