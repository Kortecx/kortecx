import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { stubProposeWorkflow } from "./fixtures/grpc-stub";
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

// 5b: the New App form's NL multi-step authoring. The console gateway serves no model, so a
// real ProposeWorkflow would honestly reject — `stubProposeWorkflow` supplies a canned
// multi-step plan so the deterministic propose→preview→author path is exercised, while the
// (unstubbed) SaveApp lands the real envelope, asserted directly via the node client.
test("New App: propose → preview → author a MULTI-STEP App (stubbed ProposeWorkflow)", async ({
  page,
}) => {
  const CANNED = {
    steps: [
      { role: "researcher", intent: "Gather the source facts from the changelog." },
      { role: "analyst", intent: "Group the changes by theme and significance." },
      { role: "writer", intent: "Write the final release notes." },
    ],
    edges: [
      { parent: 0, child: 1 },
      { parent: 1, child: 2 },
    ],
  };
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await stubProposeWorkflow(page, CANNED);
  const kx = new KxClient(gw.endpoint);

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId("new-app").click();
  await expect(page.getByTestId("new-app-form")).toBeVisible();

  // The handle is derived from the name (defaultHandle) — no handle field.
  const HANDLE = "apps/local/release-notes-writer";
  await page.getByTestId("new-app-name").fill("Release Notes Writer");
  await page.getByTestId("new-app-goal").fill("Summarize a changelog into release notes.");

  // Propose → the plan lands on the CANVAS (it used to render as a read-only <ol> you
  // could look at and not change). The structure rail reports the step count and the
  // builder shows one node per step.
  await page.getByTestId("new-app-propose").click();
  await expect(page.getByTestId("new-app-structure")).toContainText("3 steps");
  await expect(page.getByTestId("blueprint-builder")).toBeVisible();
  await expect(page.getByTestId("builder-node")).toHaveCount(3);
  // The App palette is Agent + Tool, and the workflow-only pattern macros are absent —
  // the standalone /blueprints/new route keeps all three kinds and every macro.
  await expect(page.getByTestId("builder-add-agent")).toBeVisible();
  await expect(page.getByTestId("builder-add-pure")).toHaveCount(0);
  await expect(page.getByTestId("builder-add-swarm")).toHaveCount(0);
  // The embedded canvas owns NO terminal — the form's own submit is the only save, so
  // the builder's navigating actions cannot fire from inside a half-filled form.
  await expect(page.getByTestId("builder-submit")).toHaveCount(0);
  await expect(page.getByTestId("builder-save-as-app")).toHaveCount(0);

  // Author → the SAVED envelope carries a 3-step blueprint (NOT the single-agent fallback).
  // The scaffold that follows Save needs a model and errors here — ignored, exactly like the
  // other App-authoring specs; the durable envelope is the assertion.
  await page.getByTestId("new-app-submit").click();
  await expect
    .poll(
      async () => {
        const env = (await kx.getApp(HANDLE))?.envelope as
          | { blueprint?: { steps?: unknown[] } }
          | undefined;
        return env?.blueprint?.steps?.length ?? 0;
      },
      { timeout: 30_000 },
    )
    .toBe(3);
  const stored = await kx.getApp(HANDLE);
  const steps = (stored?.envelope as { blueprint: { steps: { kind: string }[] } }).blueprint.steps;
  expect(steps.every((s) => s.kind === "model")).toBe(true);
  // 5c co-ship: every authored App still carries the capabilities rule.
  expect(JSON.stringify(stored?.envelope)).toContain("capabilities");
  kx.close();
});
