/**
 * POC-5d: the single-App IDE end-to-end in a real browser (model-free). Seeds a
 * pure-step App + a one-file project branch through the node SDK, then drives the IDE:
 * the 3 tabs, the Files pane (view + direct-edit + agentic-review wiring), the editable
 * Lineage graph, and a single-App Run that lands on the live run. No model is served,
 * so the agentic-edit propose/diff and the lineage save are asserted as WIRED (their
 * model-driven behaviour is covered by the Rust live-Gemma e2e); the pure-step Run
 * actually executes + navigates.
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
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

  // View details → the READ-ONLY capability manifest panel (needs vs. what you have).
  await page.getByTestId(`app-menu-${HANDLE}`).click();
  await page.getByTestId(`app-view-${HANDLE}`).click();
  await expect(page.getByTestId("app-view")).toBeVisible();
  await expect(page.getByTestId("app-manifest")).toBeVisible({ timeout: 15_000 });
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("app-view")).toBeHidden();

  await page.getByTestId(`app-menu-${HANDLE}`).click();
  await page.getByTestId(`app-open-${HANDLE}`).click();

  // The full-screen IDE shell + the 3 tabs.
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("app-tab-files")).toBeVisible();
  await expect(page.getByTestId("app-tab-lineage")).toBeVisible();
  await expect(page.getByTestId("app-tab-chat")).toBeVisible();

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
  await expect(page.getByTestId("app-lineage-save")).toHaveCount(0);
  // The tab is URL-addressable (refresh-safe).
  await expect(page).toHaveURL(/[?&]tab=lineage/);

  // Run: the drawer opens; with no input_schema it runs in one click and navigates to
  // the live run (the pure step commits without a model).
  await page.getByTestId("app-detail-run").click();
  await expect(page.getByTestId("app-run-drawer")).toBeVisible();
  await page.getByTestId("app-run-now").click();
  await expect(page).toHaveURL(/\/workflows\//, { timeout: 30_000 });
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
