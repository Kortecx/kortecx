import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("builder: author + interact + validate, then submit a PURE blueprint to a live run", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Reach the builder via the Blueprints "New blueprint" entry.
  await gotoViaPalette(page, "recipes");
  await page.getByTestId("new-blueprint").click();
  await expect(page.getByTestId("builder-canvas")).toBeVisible({ timeout: 30_000 });

  // The fresh builder seeds one Agent (MODEL) node.
  await expect(page.getByTestId("builder-node")).toHaveCount(1);

  // A MODEL step on a model-free serve is invalid (needs a model) ⇒ submit disabled.
  await expect(page.getByTestId("builder-validation")).toBeVisible();
  await expect(page.getByTestId("builder-submit")).toBeDisabled();

  // Clicking a node opens its config drawer (n8n-level config, D141.6).
  await page.getByTestId("builder-node").first().click();
  await expect(page.getByTestId("step-config-drawer")).toBeVisible();
  // Model-free ⇒ the honest "no model served" state (don't-fake-gaps).
  await expect(page.getByTestId("step-config-no-models")).toBeVisible();

  // Delete the agent and author a model-free PURE step instead.
  await page.getByTestId("step-config-delete").click();
  await expect(page.getByTestId("builder-node")).toHaveCount(0);
  await page.getByTestId("builder-add-pure").click();
  await expect(page.getByTestId("builder-node")).toHaveCount(1);
  // Adding a node auto-opens its config drawer; close it (Escape) so the toolbar
  // submit is reachable (the drawer scrim closes on click-outside).
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("step-config-drawer")).toHaveCount(0);

  // A single PURE step is valid ⇒ submit enabled. Build & run compiles + runs it.
  await expect(page.getByTestId("builder-submit")).toBeEnabled();
  await page.getByTestId("builder-submit").click();

  // The server compiled the DAG and routed us to the live run.
  await expect(page).toHaveURL(/\/workflows\//, { timeout: 30_000 });
});

test("builder: connect two agents into a chain (an edge appears)", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  // In-app navigation (a full page.goto would reset the in-memory connection).
  await gotoViaPalette(page, "recipes");
  await page.getByTestId("new-blueprint").click();
  await expect(page.getByTestId("builder-canvas")).toBeVisible({ timeout: 30_000 });

  // Add a second agent so the canvas has two nodes to wire.
  await page.getByTestId("builder-add-agent").click();
  await expect(page.getByTestId("builder-node")).toHaveCount(2);

  // Drag from the first node's source handle to the second node's target handle.
  const nodes = page.getByTestId("builder-node");
  const source = nodes.nth(0).locator(".react-flow__handle-bottom");
  const target = nodes.nth(1).locator(".react-flow__handle-top");
  await source.dragTo(target);

  // An edge now connects them (reactflow renders edge paths under .react-flow__edge).
  await expect(page.locator(".react-flow__edge")).toHaveCount(1, { timeout: 10_000 });
});
