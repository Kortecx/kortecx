import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { stubDeriveApp, stubProposeWorkflow } from "./fixtures/grpc-stub";
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

/**
 * The half the spec above never exercised: the plan actually LOWERING onto the canvas.
 *
 * `stubProposeWorkflow` has existed, fully implemented, with ZERO call sites — so
 * `proposeWorkflow` was never fired by any spec, `builder-propose-apply` was never
 * clicked, and `proposalToBuilderGraph`'s only guard was a pure-function unit test.
 * Between that function and a node on the canvas sat `applyProposal` → `applyInsert`
 * → the layout pass, none of it covered.
 *
 * This is model-FREE on purpose, so it runs in CI forever. The model's half — that a
 * real model returns an admissible plan — is a separate, opt-in concern; the lowering
 * is deterministic and deserves a deterministic gate.
 */
test("builder: a proposed plan lowers into wired canvas nodes (the apply half)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // A FAN-OUT: two independent middle steps that both feed a final step. A chain would
  // pass even if edges were dropped and the layout fell back to sequential order.
  await stubProposeWorkflow(page, {
    steps: [
      { role: "researcher", intent: "Gather the candidate engines" },
      { role: "analyst", intent: "Compare durability guarantees" },
      { role: "analyst", intent: "Compare operational cost" },
      { role: "writer", intent: "Write the comparison" },
    ],
    edges: [
      { parent: 0, child: 1 },
      { parent: 0, child: 2 },
      { parent: 1, child: 3 },
      { parent: 2, child: 3 },
    ],
  });

  await connectConsole(page, gw);
  await gotoViaPalette(page, "recipes");
  await page.getByTestId("new-blueprint").click();
  await expect(page.getByTestId("builder-canvas")).toBeVisible({ timeout: 30_000 });

  // The canvas starts empty — otherwise "4 nodes" could be true of nodes we did not add.
  const nodes = page.getByTestId("builder-node");
  const before = await nodes.count();

  await page.getByTestId("builder-propose").click();
  await page.getByTestId("builder-propose-goal").fill("Compare two durable-execution engines.");
  await page.getByTestId("builder-propose-submit").click();

  // The proposal renders as a reviewable list BEFORE anything touches the canvas —
  // propose-then-confirm, not propose-and-apply.
  await expect(page.getByTestId("builder-propose-steps")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("builder-propose-step")).toHaveCount(4);
  expect(await nodes.count()).toBe(before);

  await page.getByTestId("builder-propose-apply").click();
  await expect(page.getByTestId("builder-propose-panel")).toHaveCount(0);

  // THE ASSERTION. Four proposed steps become four canvas nodes…
  await expect(nodes).toHaveCount(before + 4);
  // …every one an AGENT step (a proposed role is a model step, never a tool step that
  // would land unconfigured)…
  await expect(page.getByTestId("builder-node").first()).toHaveAttribute("data-kind", "model");
  await expect(page.getByTestId("builder-node-needs-config")).toHaveCount(0);
  // …and each node carries its ROLE as the label (the node shows the role; the intent
  // rides the prompt, asserted below).
  await expect(page.getByLabel("Agent Researcher")).toBeVisible();
  await expect(page.getByLabel("Agent Writer")).toBeVisible();
  await expect(page.getByLabel("Agent Analyst")).toHaveCount(2);

  // Applying SELECTS the first inserted node, so its config drawer is already open —
  // the same landing behaviour the pattern macros have. That is where the INTENT is
  // visible, and asserting it here is what stops four correctly-labelled but EMPTY
  // agents passing everything above.
  const drawer = page.getByTestId("step-config-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer).toContainText("Gather the candidate engines");

  // The last step's intent survived too — checked after dismissing the drawer, whose
  // scrim would otherwise swallow the click.
  await page.keyboard.press("Escape");
  await expect(drawer).toHaveCount(0);
  await page.getByLabel("Agent Writer").click();
  await expect(drawer).toBeVisible();
  await expect(drawer).toContainText("Write the comparison");
});

// The Apps chat surface's multi-step authoring, end to end and model-free. The console
// gateway serves no model, so a real `DeriveApp` would honestly reject — `stubDeriveApp`
// supplies a canned design so the derive→review→approve path is exercised, while the
// (unstubbed) SaveApp lands the REAL envelope, asserted directly via the node client.
//
// ★ The design here is a FAN-OUT carrying a per-step TOOL GRANT, because those are the two
// things this surface added and neither was previously reachable: `ProposeWorkflow` could
// only ever return an empty `tool_contract` (every authoring role resolves to a pure model
// recipe), and its contract taught a chain. Asserting a sequential, tool-less plan would pass
// against the surface this replaced.
test("New App: design → review → approve a PARALLEL, tool-granted App (stubbed DeriveApp)", async ({
  page,
}) => {
  const DESIGN = {
    name: "Release Notes Writer",
    description: "Summarize a changelog into release notes.",
    steps: [
      {
        role: "researcher",
        intent: "Gather the source facts from the changelog.",
        toolContract: { "fs-read": "1" },
      },
      { role: "analyst", intent: "Group the changes by theme and significance." },
      { role: "writer", intent: "Write the final release notes." },
    ],
    // 0 and 1 have NO parent: they run at the same time and 2 joins them.
    edges: [
      { parent: 0, child: 2 },
      { parent: 1, child: 2 },
    ],
  };
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await stubDeriveApp(page, DESIGN);
  const kx = new KxClient(gw.endpoint);

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId("new-app").click();
  await expect(page.getByTestId("new-app-form")).toBeVisible();

  // The handle is derived from the design's name (defaultHandle) — no handle field.
  const HANDLE = "apps/local/release-notes-writer";
  await page.getByTestId("new-app-prompt").fill("Summarize a changelog into release notes.");

  // Design → the workflow lands on the CANVAS as an EDITABLE graph, and nothing has been
  // created. The review rail reports the step count and the builder shows one node per step.
  await page.getByTestId("new-app-derive").click();
  await expect(page.getByTestId("new-app-review")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("new-app-name")).toHaveValue("Release Notes Writer");
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

  // The tool the DESIGN asked for is on the NODE that asked for it — capabilities live on the
  // graph now, so there is no rail beside the canvas. Open step 0 and confirm its own tool is
  // granted there and NOT on the join step.
  await page.getByTestId("builder-node").first().click();
  await expect(page.getByTestId("step-config-drawer")).toBeVisible();
  await expect(page.getByTestId("step-config-agent-tools")).toContainText("fs-read");
  await page.keyboard.press("Escape");
  // And there are no app-level capability rails at all — the whole point of this change.
  await expect(page.getByTestId("new-app-tools")).toHaveCount(0);
  await expect(page.getByTestId("new-app-skills")).toHaveCount(0);
  await expect(page.getByTestId("new-app-connections")).toHaveCount(0);

  // Approve → the SAVED envelope carries a 3-step blueprint (NOT the single-agent fallback).
  // The scaffold that follows Save needs a model and errors here — ignored, exactly like the
  // other App-authoring specs; the durable envelope is the assertion.
  await page.getByTestId("new-app-approve").click();
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
  const bp = (
    (stored as { envelope: unknown }).envelope as {
      blueprint: {
        steps: { kind: string; tool_contract?: Record<string, string> }[];
        edges?: { parent: number; child: number }[];
      };
    }
  ).blueprint;
  expect(bp.steps.every((s) => s.kind === "model")).toBe(true);
  // ★ The SHAPE survived to the durable envelope. Two steps with no incoming edge is the
  // parallelism; a blueprint that had silently linearised would still have 3 steps and read
  // as a pass, which is why this asserts the edges and not the count.
  const withParent = new Set((bp.edges ?? []).map((e) => e.child));
  expect([0, 1].every((i) => !withParent.has(i))).toBe(true);
  expect(withParent.has(2)).toBe(true);
  // ★ The derived tool grant survived to the envelope ON THE STEP THAT ASKED — the per-node
  // truth, not an app-level wish. Step 0 carries it; the joining step does not.
  expect(bp.steps[0]?.tool_contract).toEqual({ "fs-read": "1" });
  expect(bp.steps[2]?.tool_contract ?? {}).toEqual({});
  // 5c co-ship: every authored App still carries the capabilities rule.
  expect(JSON.stringify(stored?.envelope)).toContain("capabilities");
  kx.close();
});
