/**
 * RULE 41 — the real agentic-scenario proof for the granular App Lineage, against the
 * MODEL-SERVED build: the `just review-serve-gemma` console on :8888 backed by a live
 * Gemma-4-12B (never Qwen3 — a Qwen3 pass is a false green).
 *
 * This is NOT the model-free e2e (`app-lineage-detail.spec.ts`, which spawns its own
 * throwaway gateway). It drives the REAL serve against a REAL App — authored through the
 * live gateway, carrying real tool contracts, an attached skill, a connection and an App
 * model_route — and asserts the diagram renders THAT App's actual bindings: the real
 * model_route deferral, the real per-step tool wishes, the real authored budget, and the
 * entry badge on the real root agent step.
 *
 * WHAT THIS DOES *NOT* PROVE (stated so the proof is not read as more than it is): it does
 * not execute the App, so it does not check the ⚑ entry badge against a live `RunApp`
 * fold. `entryAgenticStepId` mirrors the server's `entry_agentic_step_index`, and the
 * mirror is guarded instead by unit tests that reproduce the Rust test table case-for-case
 * (`lineage-step-view.test.ts`) — a static guard, not a live one. Closing that last gap
 * needs an App whose rails actually resolve: RunApp fail-closes here on
 * `missing integration: echo-cred` (and a skill's `instructions_ref` is CAS-checked
 * fail-closed), which is itself evidence the declared rails are real and enforced. That
 * follow-up is recorded in the corpus rather than faked here.
 *
 * Run: `just review-serve-gemma` in another shell, then
 *   KX_RULE41=1 npx playwright test e2e/rule41-lineage-gemma.spec.ts
 */

import { KxClient } from "@kortecx/sdk/node";
import { type Page, expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";

const CONSOLE = "http://127.0.0.1:8888";
const GRPC = "http://127.0.0.1:50151";
const HANDLE = "apps/local/rule41-research-desk";
const CAS_REF = "b".repeat(64);

// Opt-in: this spec needs the hand-started Gemma serve, so it must never run in the
// default suite (or in CI, which has no :8888 and no Gemma).
test.skip(!process.env.KX_RULE41, "Rule-41 proof: needs `just review-serve-gemma` on :8888");

test.use({ baseURL: CONSOLE });

/**
 * The complex App, authored against the LIVE gateway: 5 steps, fan-out → fan-in, a real
 * registered tool, an attached skill carrying its own tool wish, a connection, and an App
 * model_route. `s0` is the only ROOT model step ⇒ the server's entry.
 */
async function seedOnLiveGemma(): Promise<void> {
  const kx = new KxClient(GRPC);
  await kx.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Rule-41 Research Desk",
      description: "A live-Gemma multi-step App with real tools, a skill and a connection.",
      blueprint: {
        seed: 0,
        steps: [
          {
            kind: "model",
            prompt: "Plan the research\nName the sub-questions to answer.",
            max_turns: 8,
            max_tool_calls: 6,
          },
          { kind: "model", prompt: "Search for supporting sources.", model_id: "" },
          { kind: "tool", tool_contract: { "mcp-echo/echo": "1" }, args: { text: "probe" } },
          { kind: "pure" },
          { kind: "model", prompt: "Write the final report for the reader." },
        ],
        edges: [
          { parent: 0, child: 1 },
          { parent: 0, child: 2 },
          { parent: 1, child: 3 },
          { parent: 2, child: 3 },
          { parent: 3, child: 4 },
        ],
      },
      references: {
        skills: [{ name: "summarize", instructions_ref: CAS_REF, tools: { "mcp-echo/echo": "1" } }],
        connections: [{ descriptor: "mcp-echo", credential_ref: "echo-cred" }],
      },
      steering_config: {
        model: { model_route: "kx-serve:gemma" },
        tools: { requested_grants: { "mcp-echo/echo": "1" } },
      },
    },
    { handle: HANDLE },
  );
  await kx.createBranch(HANDLE);
  kx.close();
}

/** Navigate IN-APP: a `page.goto` after connecting drops the in-memory token and bounces
 *  back to the connect gate, so the catalog is reached through the ⌘K palette. */
async function openLineage(page: Page): Promise<void> {
  await connectConsole(page, { endpoint: GRPC } as never);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId(`app-menu-${HANDLE}`).click();
  await page.getByTestId(`app-open-${HANDLE}`).click();
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("app-tab-lineage").click();
  await expect(page.getByTestId("app-lineage")).toBeVisible({ timeout: 30_000 });
}

test("Rule 41: the live-Gemma App's Lineage shows each step's REAL bindings", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 1500 });
  await seedOnLiveGemma();
  await openLineage(page);

  await expect(page.getByTestId("app-lineage-diagram")).toHaveAttribute("data-steps", "5");

  // Real per-step detail, read off the App the live gateway stored.
  await expect(page.getByTestId("lineage-node-s0")).toContainText("Plan the research");
  await expect(page.getByTestId("lineage-node-s4")).toContainText("Write the final report");
  // No step pins a model ⇒ every agent step defers to the App's real model_route.
  await expect(page.getByTestId("lineage-model-s0")).toHaveText("inherits kx-serve:gemma");
  await expect(page.getByTestId("lineage-meta-s0")).toHaveText("8 turns · 6 calls");
  await expect(page.getByTestId("lineage-tools-s2")).toContainText("mcp-echo/echo");
  await expect(page.getByTestId("lineage-node-s3")).toContainText("Step 4");

  // THE claim: the entry badge is on s0 — the first ROOT model step.
  await expect(page.getByTestId("lineage-entry-s0")).toBeVisible();
  await expect(page.getByTestId("lineage-entry-s1")).toHaveCount(0);
  await expect(page.getByTestId("lineage-entry-s4")).toHaveCount(0);

  for (const theme of ["light", "dark"] as const) {
    const current = await page.locator("html").getAttribute("data-theme");
    if (current !== theme) {
      await page.getByTestId("theme-toggle").click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const shot = testInfo.outputPath(`rule41-lineage-${theme}.png`);
    await page.screenshot({ path: shot, fullPage: true });
    await testInfo.attach(`rule41-lineage-${theme}`, { path: shot, contentType: "image/png" });
  }
});
