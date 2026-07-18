/**
 * POC-5d: the single-App IDE end-to-end in a real browser (model-free). Seeds a
 * pure-step App + a one-file project branch through the node SDK, then drives the IDE:
 * the 3 tabs, the Files pane (view + direct-edit + agentic-review wiring), the editable
 * Lineage graph, and a single-App Run that lands on the live run. No model is served,
 * so the agentic-edit propose/diff and the lineage save are asserted as WIRED (their
 * model-driven behaviour is covered by the Rust live-Gemma e2e); the pure-step Run
 * actually executes + navigates.
 *
 * The SECOND test (item6) turns the unified agentic-MODIFY drawer into a real red/green
 * BEHAVIOURAL net over a MULTI-ARTIFACT edit with ROLLBACK: the model-inference RPCs are
 * stubbed model-free (`stubReactEdit`) while every branch mutation stays REAL, so ONE
 * high-level instruction across TWO files is proven to advance BOTH manifest paths, and a
 * rollback is proven to restore each to its distinct prior body. Each assertion reads a
 * consequence of real server state (not a rendered node), so deleting the approve or the
 * rollback wiring can't stay green.
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

  // The Modify header action opens the unified agentic-modify drawer (describe a change →
  // multi-file diff → approve/rollback); close it before continuing.
  await page.getByTestId("app-detail-chat").click();
  await expect(page.getByTestId("app-chat-drawer")).toBeVisible();
  await expect(page.getByTestId("app-edit-propose")).toBeVisible();
  // The scrim covers the viewport (a real dim users can click to dismiss) — clicking
  // its top-left (clear of the right-side drawer) closes the drawer.
  const scrimBox = await page.getByLabel("Close modify drawer").boundingBox();
  expect(scrimBox?.height ?? 0).toBeGreaterThan(100);
  await page.getByLabel("Close modify drawer").click({ position: { x: 20, y: 20 } });
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

test("App IDE unified modify (item6): a coherent MULTI-artifact diff + rollback is a real behavioural net (model-free)", async ({
  page,
}) => {
  const EDIT_HANDLE = "apps/local/edit-net";
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // Seed the App + a TWO-file branch (distinct bodies), AND pre-seed the agent's "proposed"
  // rewrite as a real content-store blob, so approve can advance the REAL manifest to a REAL
  // ref for each file. The two originals differ so rollback is proven to restore each file to
  // its OWN prior body (its distinct CAS blob is still present).
  const README_ORIGINAL = "# Readme\nhello from the IDE.\n";
  const CONFIG_ORIGINAL = '{ "name": "edit-net", "widget": "old" }\n';
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
  const readme = await seed.putContent(new TextEncoder().encode(README_ORIGINAL), {
    filename: "README.md",
  });
  await seed.advanceBranch(EDIT_HANDLE, "README.md", readme.contentRef);
  const config = await seed.putContent(new TextEncoder().encode(CONFIG_ORIGINAL), {
    filename: "config.json",
  });
  await seed.advanceBranch(EDIT_HANDLE, "config.json", config.contentRef);
  const proposedBytes = new TextEncoder().encode(
    "AGENT_EDIT_MARKER: coherent rename applied by the agent.\n",
  );
  const proposed = await seed.putContent(proposedBytes, { filename: "edit.txt" });

  // Stub ONLY the model-inference RPCs of editBranchPropose (react-edit → projection →
  // content); GetBranch/GetBranchContent/AdvanceBranch stay REAL. The stub returns the same
  // proposed blob for every react-edit invoke, so each of the two per-file proposals resolves
  // to a real ref and each approve/rollback hits the real branch store.
  await stubReactEdit(page, { resultRef: proposed.contentRef, proposedBytes });

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId(`app-menu-${EDIT_HANDLE}`).click();
  await page.getByTestId(`app-open-${EDIT_HANDLE}`).click();
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });

  // Open the unified modify drawer: ONE high-level instruction, TWO files in scope.
  await page.getByTestId("app-detail-chat").click();
  await expect(page.getByTestId("app-chat-drawer")).toBeVisible();
  await page.getByTestId("app-edit-instruction").fill("rename the widget to Gadget everywhere");
  await page.getByTestId("app-edit-file-README.md").check();
  await page.getByTestId("app-edit-file-config.json").check();
  await page.getByTestId("app-edit-propose").click();

  // BEHAVIOUR 1: BOTH per-file propose composites ran (GetBranch → Invoke → GetProjection →
  // GetContent → GetBranchContent, once per file) and each produced a diff whose proposed ≠
  // current, so the single review gate opens with a diff for EACH file and approve enables.
  await expect(page.getByTestId("app-edit-review")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("app-edit-diff-README.md")).toBeVisible();
  await expect(page.getByTestId("app-edit-diff-config.json")).toBeVisible();
  await expect(page.getByTestId("app-edit-approve")).toBeEnabled();

  // BEHAVIOUR 2: approve applies against the REAL gateway (unstubbed AdvanceBranch, once per
  // file); on success the review gate closes and the applied/rollback panel opens.
  await page.getByTestId("app-edit-approve").click();
  await expect(page.getByTestId("app-edit-review")).toHaveCount(0, { timeout: 15_000 });
  await expect(page.getByTestId("app-edit-applied")).toBeVisible();
  await expect(page.getByTestId("app-edit-rollback")).toBeVisible();

  // BEHAVIOUR 3 (ground truth): the REAL branch manifest now resolves BOTH files to the
  // approved body — a genuine multi-artifact modify (read straight from the gateway; the
  // node client bypasses the browser stub).
  for (const path of ["README.md", "config.json"]) {
    await expect
      .poll(
        async () => {
          const body = await seed.getBranchContent(EDIT_HANDLE, path);
          return body ? new TextDecoder().decode(body) : "";
        },
        { timeout: 15_000 },
      )
      .toContain("AGENT_EDIT_MARKER");
  }

  // BEHAVIOUR 4 (ground truth): rollback re-advances each path to its PRIOR ref, restoring
  // each file to its OWN distinct original body — no history RPC, just the still-present CAS
  // blobs. The marker is gone from both files.
  await page.getByTestId("app-edit-rollback").click();
  await expect(page.getByTestId("app-edit-applied")).toHaveCount(0, { timeout: 15_000 });
  await expect
    .poll(
      async () => {
        const body = await seed.getBranchContent(EDIT_HANDLE, "README.md");
        return body ? new TextDecoder().decode(body) : "";
      },
      { timeout: 15_000 },
    )
    .toBe(README_ORIGINAL);
  await expect
    .poll(
      async () => {
        const body = await seed.getBranchContent(EDIT_HANDLE, "config.json");
        return body ? new TextDecoder().decode(body) : "";
      },
      { timeout: 15_000 },
    )
    .toBe(CONFIG_ORIGINAL);
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
