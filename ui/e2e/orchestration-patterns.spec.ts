import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

// The blueprint builder scaffolds multi-agent orchestration patterns
// (swarm / supervisor / consensus) as clusters of the existing model/pure node
// vocabulary. These drive the toolbar macros against a live (model-free) serve and
// capture both-theme screenshots for the Rule-12/13 console review.

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

/** Open a fresh blueprint builder (it seeds one Agent node). */
async function openBuilder(page: Page) {
  await gotoViaPalette(page, "recipes");
  await page.getByTestId("new-blueprint").click();
  await expect(page.getByTestId("builder-canvas")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("builder-node")).toHaveCount(1); // the seed agent
}

test("builder: + Supervisor scaffolds planner → workers → gather (4 nodes, 4 edges)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await openBuilder(page);

  await page.getByTestId("builder-add-supervisor").click();
  // planner + 2 workers + gather = 4 new nodes on top of the seed agent.
  await expect(page.getByTestId("builder-node")).toHaveCount(5);
  // planner→worker×2 + worker→gather×2 = 4 data edges.
  await expect(page.locator(".react-flow__edge")).toHaveCount(4, { timeout: 10_000 });

  // Inserting a pattern auto-opens the first node's drawer; close it.
  await page.keyboard.press("Escape");
  // The MODEL nodes need a model on this model-free serve ⇒ the honest submit gate.
  await expect(page.getByTestId("builder-validation")).toBeVisible();
  await expect(page.getByTestId("builder-submit")).toBeDisabled();
});

test("builder: + Consensus · majority inserts voters + a pure vote sink (3 nodes, 2 edges)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await openBuilder(page);

  await page.getByTestId("builder-add-consensus-majority").click();
  // 2 voters + 1 pure sink = 3 new nodes on top of the seed agent.
  await expect(page.getByTestId("builder-node")).toHaveCount(4);
  await expect(page.locator(".react-flow__edge")).toHaveCount(2, { timeout: 10_000 });
});

test("builder: + Consensus · judge and + Swarm are authorable", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await openBuilder(page);

  await page.getByTestId("builder-add-swarm").click(); // 2 agents + gather
  await expect(page.getByTestId("builder-node")).toHaveCount(4);
  await page.keyboard.press("Escape");
  await page.getByTestId("builder-add-consensus-judge").click(); // 2 voters + judge
  await expect(page.getByTestId("builder-node")).toHaveCount(7);
  await expect(page.locator(".react-flow__edge")).toHaveCount(4, { timeout: 10_000 });
});

// Rule 12/13: the pattern authoring must render correctly in BOTH themes. Capture a
// supervisor cluster under light and dark; the screenshots are the console-review evidence.
test("builder: pattern authoring renders in light and dark (screenshots)", async ({
  page,
}, testInfo) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // --- light (chromium's default OS preference resolves to light) ---
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await openBuilder(page);
  await page.getByTestId("builder-add-supervisor").click();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("builder-node")).toHaveCount(5);
  const light = testInfo.outputPath("builder-supervisor-light.png");
  await page.screenshot({ path: light, fullPage: true });
  await testInfo.attach("supervisor-light", { path: light, contentType: "image/png" });

  // --- dark (toggle in Settings, then re-author in the fresh builder) ---
  await page.getByTestId("nav-settings").click();
  await page.getByTestId("theme-chip-dark").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await openBuilder(page);
  await page.getByTestId("builder-add-supervisor").click();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("builder-node")).toHaveCount(5);
  const dark = testInfo.outputPath("builder-supervisor-dark.png");
  await page.screenshot({ path: dark, fullPage: true });
  await testInfo.attach("supervisor-dark", { path: dark, contentType: "image/png" });
});
