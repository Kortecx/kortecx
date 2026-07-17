/**
 * POC-5d: the single-App IDE end-to-end in a real browser (model-free). Seeds a
 * pure-step App + a one-file project branch through the node SDK, then drives the IDE:
 * the 3 tabs, the Files pane (view + direct-edit + agentic-review wiring), the editable
 * Lineage graph, and a single-App Run that lands on the live run. No model is served,
 * so the agentic-edit propose/diff and the lineage save are asserted as WIRED (their
 * model-driven behaviour is covered by the Rust live-Gemma e2e); the pure-step Run
 * actually executes + navigates.
 *
 * The SECOND test turns that WIRED-only propose→diff→approve gate into a real red/green
 * BEHAVIOURAL net: the model-inference RPCs are stubbed model-free (`stubReactEdit`) while
 * the branch mutation stays REAL, so `approve` is proven to advance the actual manifest —
 * a regression in the propose or approve wiring fails a check (deleting an assertion can't
 * stay green, because each asserts a consequence of real server state, not a rendered node).
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { stubReactEdit } from "./fixtures/grpc-stub";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

const HANDLE = "apps/local/ide-demo";

test("App IDE (POC-5d): tabs, file view + edit wiring, lineage, and a single-App run", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // Seed a pure-step App + a one-file project branch (the App handle IS the branch
  // handle — one-App-one-branch). Pure so the Run leg executes model-free.
  const seed = new KxClient(gw.endpoint);
  const envelope = {
    schema: "kortecx.app/v1",
    name: "IDE Demo",
    blueprint: { seed: 0, steps: [{ kind: "pure", params: { note: "demo" } }] },
  };
  await seed.saveApp(envelope, { handle: HANDLE });
  await seed.createBranch(HANDLE);
  const put = await seed.putContent(new TextEncoder().encode("# Readme\nhello from the IDE.\n"), {
    filename: "README.md",
  });
  await seed.advanceBranch(HANDLE, "README.md", put.contentRef);
  seed.close();

  await connectConsole(page, gw);

  // Reach the IDE via the Apps section → the card's overflow menu → Open project.
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();

  // Clicking the App's NAME opens the READ-ONLY View popover — a glimpse of the App
  // (summary + capability manifest); deep edit opens in a new tab from here.
  await page.getByTestId(`app-card-view-${HANDLE}`).click();
  await expect(page.getByTestId("app-view")).toBeVisible();
  await expect(page.getByTestId("app-manifest")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId(`app-view-open-tab-${HANDLE}`)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("app-view")).toBeHidden();

  await page.getByTestId(`app-menu-${HANDLE}`).click();
  await page.getByTestId(`app-open-${HANDLE}`).click();

  // The full-screen IDE shell + the tabs (Chat is a HEADER ACTION now, not a tab).
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("app-tab-files")).toBeVisible();
  await expect(page.getByTestId("app-tab-lineage")).toBeVisible();
  await expect(page.getByTestId("app-tab-tools")).toBeVisible();
  await expect(page.getByTestId("app-tab-integrations")).toBeVisible();
  await expect(page.getByTestId("app-tab-chat")).toHaveCount(0);

  // The Chat header action opens the agentic "Chat & edit" drawer (converse + the
  // propose→diff→approve edit gate); close it before continuing.
  await page.getByTestId("app-detail-chat").click();
  await expect(page.getByTestId("app-chat-drawer")).toBeVisible();
  await expect(page.getByTestId("app-edit-propose")).toBeVisible();
  // The scrim covers the viewport (a real dim users can click to dismiss) — clicking
  // its top-left (clear of the right-side drawer) closes the drawer.
  const scrimBox = await page.getByLabel("Close chat").boundingBox();
  expect(scrimBox?.height ?? 0).toBeGreaterThan(100);
  await page.getByLabel("Close chat").click({ position: { x: 20, y: 20 } });
  await expect(page.getByTestId("app-chat-drawer")).toHaveCount(0);

  // Files: the tree is a collapsible sidebar rail (collapse hides it, expand restores it).
  await expect(page.getByTestId("app-files-sidebar")).toBeVisible();
  await page.getByTestId("app-files-collapse").click();
  await expect(page.getByTestId("file-README.md")).toHaveCount(0);
  await page.getByTestId("app-files-collapse").click();
  await expect(page.getByTestId("file-README.md")).toBeVisible();

  // Files: select README → the viewer renders → direct-edit + agentic-review affordances.
  await page.getByTestId("file-README.md").click();
  await expect(page.getByTestId("app-file-edit-direct")).toBeVisible();
  await expect(page.getByTestId("app-file-edit-agentic")).toBeVisible();
  // Direct edit mounts the editable editor + Save (Save is disabled until dirty).
  await page.getByTestId("app-file-edit-direct").click();
  await expect(page.getByTestId("app-file-direct-editor")).toBeVisible();
  await expect(page.getByTestId("app-file-save")).toBeDisabled();
  await page.getByTestId("app-file-cancel").click();
  // Agentic review: the propose form is wired (no model served → we don't fire it).
  await page.getByTestId("app-file-edit-agentic").click();
  await expect(page.getByTestId("app-file-edit-instruction")).toBeVisible();
  await expect(page.getByTestId("app-file-propose")).toBeVisible();
  await page.getByTestId("app-file-cancel").click();

  // Lineage: a READ-ONLY view of the App's structure (authoring lives in the builder).
  await page.getByTestId("app-tab-lineage").click();
  await expect(page.getByTestId("app-lineage")).toBeVisible();
  await expect(page.getByTestId("lineage-readonly-notice")).toBeVisible();
  // A clean static diagram (dagre node cards + SVG connectors), not a reactflow editor.
  await expect(page.getByTestId("app-lineage-diagram")).toBeVisible();
  await expect(page.getByTestId("app-lineage-save")).toHaveCount(0);
  // The tab is URL-addressable (refresh-safe).
  await expect(page).toHaveURL(/[?&]tab=lineage/);

  // MCP Tools: the EDITABLE rail — attaching a registered tool persists via SaveApp
  // (a wish, granted only at run) and it appears in the attached row; detach reverts.
  await page.getByTestId("app-tab-tools").click();
  await expect(page.getByTestId("app-tools-rail")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("app-tools-attached")).toContainText("No tools attached");
  if ((await page.getByTestId("app-tools-attachable").count()) > 0) {
    const firstTool = page.getByTestId("app-tools-attachable").locator("button").first();
    const toolName = ((await firstTool.textContent()) ?? "").replace(/^\+\s*/, "").trim();
    await firstTool.click();
    await expect(page.getByTestId(`app-tool-detach-${toolName}`)).toBeVisible({ timeout: 15_000 });
    await page.getByTestId(`app-tool-detach-${toolName}`).click();
    await expect(page.getByTestId("app-tools-attached")).toContainText("No tools attached", {
      timeout: 15_000,
    });
  }

  // Integrations: the EDITABLE rail — bind a connector by endpoint + credential NAME
  // (never the secret) → it persists; unbind reverts.
  await page.getByTestId("app-tab-integrations").click();
  await expect(page.getByTestId("app-tab-integrations")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("app-connections-rail")).toBeVisible();
  await expect(page.getByTestId("app-connections-attached")).toContainText("No integrations bound");
  await page.getByTestId("app-connection-descriptor").fill("https://mcp.example/sse");
  await page.getByTestId("app-connection-credential").fill("EXAMPLE_TOKEN");
  await page.getByTestId("app-connection-bind").click();
  const bound = page.getByTestId("app-connection-detach-https://mcp.example/sse");
  await expect(bound).toBeVisible({ timeout: 15_000 });
  await expect(bound).toContainText("EXAMPLE_TOKEN");
  await bound.click();
  await expect(page.getByTestId("app-connections-attached")).toContainText(
    "No integrations bound",
    {
      timeout: 15_000,
    },
  );

  // Run: the drawer opens; with no input_schema it runs in one click and navigates to
  // the live run (the pure step commits without a model).
  await page.getByTestId("app-detail-run").click();
  await expect(page.getByTestId("app-run-drawer")).toBeVisible();
  await page.getByTestId("app-run-now").click();
  await expect(page).toHaveURL(/\/workflows\//, { timeout: 30_000 });
});

test("App IDE agentic edit: the propose→diff→approve gate is a real behavioural net (model-free)", async ({
  page,
}) => {
  const EDIT_HANDLE = "apps/local/edit-net";
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // Seed the App + a one-file branch, AND pre-seed the agent's "proposed" rewrite as a real
  // content-store blob, so approve can advance the REAL manifest to a REAL ref.
  const seed = new KxClient(gw.endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Edit Net",
      blueprint: { seed: 0, steps: [{ kind: "pure", params: { note: "demo" } }] },
    },
    { handle: EDIT_HANDLE },
  );
  await seed.createBranch(EDIT_HANDLE);
  const original = await seed.putContent(
    new TextEncoder().encode("# Readme\nhello from the IDE.\n"),
    {
      filename: "README.md",
    },
  );
  await seed.advanceBranch(EDIT_HANDLE, "README.md", original.contentRef);
  const proposedBytes = new TextEncoder().encode(
    "# Readme\n\nAGENT_EDIT_MARKER: validation note added by the agent.\n",
  );
  const proposed = await seed.putContent(proposedBytes, { filename: "README.md" });

  // Stub ONLY the model-inference RPCs of editBranchPropose; AdvanceBranch/GetBranchContent stay real.
  await stubReactEdit(page, { resultRef: proposed.contentRef, proposedBytes });

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId(`app-menu-${EDIT_HANDLE}`).click();
  await page.getByTestId(`app-open-${EDIT_HANDLE}`).click();
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });

  // Open the Chat & edit drawer and drive the gate for real.
  await page.getByTestId("app-detail-chat").click();
  await expect(page.getByTestId("app-chat-drawer")).toBeVisible();
  await page.getByTestId("app-edit-target").selectOption("README.md");
  await page.getByTestId("app-edit-instruction").fill("add a validation note");
  await page.getByTestId("app-edit-propose").click();

  // BEHAVIOUR 1: the whole propose composite ran (GetBranch → Invoke → GetProjection →
  // GetContent → GetBranchContent) and produced a diff whose proposed ≠ current, so the
  // review gate opens and approve enables. A bare "the button is visible" check can't see this.
  await expect(page.getByTestId("app-edit-review")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("app-edit-diff")).toBeVisible();
  await expect(page.getByTestId("app-edit-approve")).toBeEnabled();

  // BEHAVIOUR 2: approve applies against the REAL gateway (unstubbed AdvanceBranch); on
  // success the drawer resets the proposal and the review gate closes.
  await page.getByTestId("app-edit-approve").click();
  await expect(page.getByTestId("app-edit-review")).toHaveCount(0, { timeout: 15_000 });

  // BEHAVIOUR 3 (ground truth): the REAL branch manifest now resolves README.md to the
  // approved body — read straight from the gateway (the node client bypasses the browser stub).
  await expect
    .poll(
      async () => {
        const body = await seed.getBranchContent(EDIT_HANDLE, "README.md");
        return body ? new TextDecoder().decode(body) : "";
      },
      { timeout: 15_000 },
    )
    .toContain("AGENT_EDIT_MARKER");
  seed.close();
});

test("Workflows → Apps (WAVE-3): the catalog links to the Apps section (Apps have one home)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  const seed = new KxClient(gw.endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Trigger Demo",
      blueprint: { seed: 0, steps: [{ kind: "pure", params: { note: "x" } }] },
    },
    { handle: "apps/local/trigger-demo" },
  );
  seed.close();

  await connectConsole(page, gw);
  await gotoViaPalette(page, "runs");
  await expect(page.getByTestId("runs-section")).toBeVisible();
  // WAVE-3: saved Apps are no longer duplicated in the Workflows catalog — the
  // catalog links to the Apps section, where an App runs from its typed drawer.
  await expect(page.getByTestId("runs-apps")).toHaveCount(0);
  await page.getByTestId("workflows-apps-link").click();
  await expect(page).toHaveURL(/\/apps$/);
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await expect(page.getByTestId("app-run-apps/local/trigger-demo")).toBeVisible({
    timeout: 30_000,
  });
});
