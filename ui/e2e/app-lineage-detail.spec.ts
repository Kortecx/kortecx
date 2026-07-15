/**
 * C1 — the App Lineage diagram's GRANULAR per-step detail, end-to-end in a real browser.
 *
 * Reviewed against a COMPLEX App, not a toy: the repo's App fixtures are all single-step,
 * which is exactly why the coarse card survived so long — one node reads fine. This spec
 * seeds a 6-step research App (fan-out → fan-in, mixed model/tool/pure, per-step tool
 * contracts + budgets, attached skills + connections + an App model_route) through the
 * real node SDK against a live gateway, then asserts each card carries its OWN step's
 * detail — the thing 6 identical "Agent" cards could never show.
 *
 * Model-free: SaveApp + the diagram are a read path, so no model need be served. The real
 * agentic proof that these bindings are what the run actually uses is the Gemma serve
 * (Rule 41). Both themes per D142.1/GR13, and a screenshot gallery is attached as review
 * evidence (Rule 12/13) — the Lineage pane had no visual coverage at all before this.
 */

import { KxClient } from "@kortecx/sdk/node";
import { type Page, expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

const HANDLE = "apps/local/lineage-detail";
const CAS_REF = "a".repeat(64);

/**
 * A realistic multi-step agentic App:
 *
 *   s0 plan (root agent — the ENTRY step the App's skills + tool wishes fold onto)
 *    ├─ s1 search   (agentic: web/search + web/fetch, budget 8/6)
 *    └─ s2 archive  (agentic: gmail/search, budget 4/3)
 *         s3 fetch  (a TOOL step firing one registered tool)
 *    └─ s4 reconcile (pure — no model, no tools: the degradation case)
 *         s5 report (agent naming NO model ⇒ inherits the App's model_route)
 */
async function seedComplexApp(endpoint: string): Promise<void> {
  const seed = new KxClient(endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Research Desk",
      description: "A multi-step research App with real tools, skills and connections.",
      blueprint: {
        seed: 0,
        steps: [
          {
            kind: "model",
            model_id: "gemma-4-12b",
            prompt: "Plan the research\nBreak the question into independent sub-questions.",
            tool_contract: { "web/search": "1" },
            max_turns: 8,
            max_tool_calls: 6,
          },
          {
            kind: "model",
            model_id: "gemma-4-12b",
            prompt: "Search the open web for supporting sources.",
            tool_contract: { "web/search": "1", "web/fetch": "1", "mcp-echo/echo": "1" },
            max_turns: 8,
            max_tool_calls: 6,
          },
          {
            kind: "model",
            model_id: "gemma-4-12b",
            prompt: "Search the mail archive for prior context.",
            tool_contract: { "gmail/search": "1" },
            max_turns: 4,
            max_tool_calls: 3,
          },
          { kind: "tool", tool_contract: { "web/fetch": "1" }, args: { url: "https://x.test" } },
          { kind: "pure" },
          { kind: "model", prompt: "Write the final report for the reader." },
        ],
        edges: [
          { parent: 0, child: 1 },
          { parent: 0, child: 2 },
          { parent: 1, child: 3 },
          { parent: 2, child: 4 },
          { parent: 3, child: 4 },
          { parent: 4, child: 5 },
        ],
      },
      references: {
        skills: [{ name: "summarize", instructions_ref: CAS_REF, tools: { "web/search": "1" } }],
        connections: [{ descriptor: "gmail", credential_ref: "gmail-oauth" }],
      },
      steering_config: {
        model: { model_route: "kx-serve:gemma" },
        tools: { requested_grants: { "web/search": "1" } },
      },
    },
    { handle: HANDLE },
  );
  await seed.createBranch(HANDLE);
  seed.close();
}

/** An App whose skills have nowhere to land: a PURE root ahead of the only agent step. */
async function seedNoEntryApp(endpoint: string, handle: string): Promise<void> {
  const seed = new KxClient(endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "No Entry",
      blueprint: {
        seed: 0,
        steps: [{ kind: "pure" }, { kind: "model", prompt: "Summarize the input." }],
        edges: [{ parent: 0, child: 1 }],
      },
      references: { skills: [{ name: "summarize", instructions_ref: CAS_REF }] },
    },
    { handle },
  );
  await seed.createBranch(handle);
  seed.close();
}

async function openLineage(page: Page, handle: string): Promise<void> {
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId(`app-menu-${handle}`).click();
  await page.getByTestId(`app-open-${handle}`).click();
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("app-tab-lineage").click();
  await expect(page.getByTestId("app-lineage")).toBeVisible();
}

test("every step's card carries its OWN model, requested tools and budget", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await seedComplexApp(gw.endpoint);
  await connectConsole(page, gw);
  await openLineage(page, HANDLE);

  await expect(page.getByTestId("app-lineage-diagram")).toHaveAttribute("data-steps", "6");

  // DISTINCT titles derived from each step's prompt — the whole point. Before C1 these
  // six cards all read "Agent".
  await expect(page.getByTestId("lineage-node-s0")).toContainText("Plan the research");
  await expect(page.getByTestId("lineage-node-s1")).toContainText("Search the open web");
  await expect(page.getByTestId("lineage-node-s2")).toContainText("Search the mail archive");
  await expect(page.getByTestId("lineage-node-s5")).toContainText("Write the final report");

  // Per-step model binding.
  await expect(page.getByTestId("lineage-model-s0")).toHaveText("gemma-4-12b");
  // s5 names no model ⇒ it binds the App's model_route at run. Say so, don't show blank.
  await expect(page.getByTestId("lineage-model-s5")).toHaveText("inherits kx-serve:gemma");

  // Per-step tool wishes — and the overflow count when a step requests more than fits.
  await expect(page.getByTestId("lineage-tools-s0")).toContainText("requests");
  await expect(page.getByTestId("lineage-tools-s0")).toContainText("web/search");
  await expect(page.getByTestId("lineage-tools-s1")).toContainText("+1");
  await expect(page.getByTestId("lineage-tools-s2")).toContainText("gmail/search");

  // Per-step budgets differ, and are only ever what the blueprint authored.
  await expect(page.getByTestId("lineage-meta-s0")).toHaveText("8 turns · 6 calls");
  await expect(page.getByTestId("lineage-meta-s2")).toHaveText("4 turns · 3 calls");
  // s5 authored no budget ⇒ none is invented for it.
  await expect(page.getByTestId("lineage-meta-s5")).toHaveCount(0);

  // The pure step degrades: no model, no tools, no budget — just its ordinal.
  await expect(page.getByTestId("lineage-node-s4")).toContainText("Step 5");
  await expect(page.getByTestId("lineage-model-s4")).toHaveCount(0);
  await expect(page.getByTestId("lineage-tools-s4")).toHaveCount(0);

  // The TOOL step names the tool it fires.
  await expect(page.getByTestId("lineage-node-s3")).toContainText("web/fetch");
});

test("the entry badge marks the one step the App's skills + tools fold onto", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await seedComplexApp(gw.endpoint);
  await connectConsole(page, gw);
  await openLineage(page, HANDLE);

  // s0 is the first model step with no inbound edge — the server's entry_agentic_step_index.
  await expect(page.getByTestId("lineage-entry-s0")).toBeVisible();
  // And ONLY s0: s1/s2/s5 are model steps too, but none is a root.
  await expect(page.getByTestId("lineage-entry-s1")).toHaveCount(0);
  await expect(page.getByTestId("lineage-entry-s5")).toHaveCount(0);
  // Nothing is dropped here, so no warning.
  await expect(page.getByTestId("lineage-fold-warning")).toHaveCount(0);
});

test("warns when an App's attached skills have no root agent step to fold onto", async ({
  page,
}) => {
  const handle = "apps/local/lineage-noentry";
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await seedNoEntryApp(gw.endpoint, handle);
  await connectConsole(page, gw);
  await openLineage(page, handle);

  // The Skills rail would show "summarize" attached; the run silently drops it. This
  // warning is the only place a user can learn that.
  await expect(page.getByTestId("lineage-fold-warning")).toContainText("can't be applied");
  await expect(page.getByTestId("lineage-entry-s0")).toHaveCount(0);
  await expect(page.getByTestId("lineage-entry-s1")).toHaveCount(0);
});

test("the granular diagram renders in light and dark (screenshots)", async ({ page }, testInfo) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await seedComplexApp(gw.endpoint);
  await connectConsole(page, gw);
  await openLineage(page, HANDLE);

  for (const theme of ["light", "dark"] as const) {
    // Toggle with no drawer open — an open scrim occludes the navbar toggle (the UI-3 gotcha).
    const current = await page.locator("html").getAttribute("data-theme");
    if (current !== theme) {
      await page.getByTestId("theme-toggle").click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    // The detail must survive the theme, not just the card outline.
    await expect(page.getByTestId("app-lineage-diagram")).toBeVisible();
    await expect(page.getByTestId("lineage-entry-s0")).toBeVisible();
    await expect(page.getByTestId("lineage-model-s0")).toBeVisible();
    await expect(page.getByTestId("lineage-tools-s1")).toBeVisible();

    const shot = testInfo.outputPath(`lineage-detail-${theme}.png`);
    await page.screenshot({ path: shot, fullPage: true });
    await testInfo.attach(`lineage-detail-${theme}`, { path: shot, contentType: "image/png" });
  }
});
