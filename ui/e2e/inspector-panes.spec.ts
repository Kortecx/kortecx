/**
 * PR-2 inspector panes: the node drawer's Prompt/Params/Tools panes resolve
 * the Mote's ADMITTED definition over the real `GetMoteDetail`, and the Inputs
 * pane joins each inbound edge to its parent's RESOLVED result text (the
 * "edge resolved text" GetContentBatch join). Pane switches must never
 * relayout the graph (the §2.184 no-thrash invariant extends to the panes).
 */

import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("inspector: def panes resolve over GetMoteDetail; Inputs resolves parent text", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // The 5-node fanout: a root with 4 children — children have inbound edges.
  await runRecipe(page, { handle: "kx/recipes/passthrough-dag" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBe(5);

  // Find a node WITH parents (the Inputs join target): open drawers until the
  // meta shows a non-zero parent count (≤5 attempts on the fanout).
  const nodes = page.getByTestId("mote-node");
  const count = await nodes.count();
  let found = false;
  for (let i = 0; i < count && !found; i++) {
    await nodes.nth(i).click();
    const drawer = page.getByTestId("node-detail-drawer");
    await expect(drawer).toBeVisible({ timeout: 15_000 });
    const parents = await drawer
      .locator("dt", { hasText: "Parents" })
      .locator("xpath=following-sibling::dd")
      .innerText();
    if (Number(parents) > 0) {
      found = true;
      break;
    }
    await page.keyboard.press("Escape");
    await expect(drawer).toBeHidden({ timeout: 15_000 });
  }
  expect(found).toBe(true);

  const drawer = page.getByTestId("node-detail-drawer");
  // Layout guard: record the FIRST node's transform before pane switches.
  const probe = nodes.first();
  const nodeTransform = () =>
    probe.evaluate((el) => {
      const node = el.closest(".react-flow__node");
      return node ? getComputedStyle(node).transform : "";
    });
  const layoutBefore = await nodeTransform();

  // Tools pane: the def resolved (step kind + nd-class + the def-hash chip).
  await drawer.getByTestId("inspector-pane-tools").click();
  await expect(drawer.getByTestId("inspector-tools")).toBeVisible({ timeout: 30_000 });
  await expect(drawer.getByTestId("inspector-tools")).toContainText("ND class");

  // Params pane (the fanout children carry def config; honest empty otherwise).
  await drawer.getByTestId("inspector-pane-params").click();
  await expect(
    drawer.getByTestId("inspector-params").or(drawer.getByText(/No params/i)),
  ).toBeVisible({ timeout: 30_000 });

  // Inputs pane: the parent's committed result RESOLVED to text.
  await drawer.getByTestId("inspector-pane-inputs").click();
  await expect(drawer.getByTestId("inspector-inputs")).toBeVisible({ timeout: 30_000 });
  const row = drawer.getByTestId("inspector-input-row").first();
  await expect(row).toContainText(/data|control/);
  // The resolved text renders in the read-only Monaco viewer (D141.2).
  await expect(row.locator(".monaco-editor")).toBeVisible({ timeout: 30_000 });

  // The graph did NOT relayout across pane switches (no-thrash preserved).
  expect(await nodeTransform()).toBe(layoutBefore);
});
