/**
 * T-RUNAPP-CONTEXT-RAIL e2e: the "New App" authoring form exposes the declarative
 * knowledge rail — a "Ground on dataset" chip (over the live ListDatasets) + a
 * guidance rule — and the saved App envelope carries `references.datasets` +
 * `references.rules`. At RunApp those resolve server-side into a `retrieve@1` grant
 * (self-grounding) + an entry-step context item. Model-free: the dataset is seeded
 * via the SDK's FFI-free client-vector path (no Metal), and we assert the SAVED
 * envelope (not the model scaffold, which needs a served model).
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

test("New App: author a grounded App (dataset chip + guidance rule) and the rail is saved", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // Seed a corpus via the FFI-free client-vector path (no model / no Metal) so the
  // "Ground on dataset" chip has a non-empty dataset to offer.
  const kx = new KxClient(gw.endpoint);
  await kx.ingestDocuments("research", [
    { content: new TextEncoder().encode("alpha"), embedding: [1, 0, 0, 0.1] },
    { content: new TextEncoder().encode("bravo"), embedding: [0, 1, 0, 0.1] },
  ]);

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();

  // Open the inline New App authoring panel.
  await page.getByTestId("new-app").click();
  await expect(page.getByTestId("new-app-form")).toBeVisible();

  // The declarative rail is present: the "Ground on dataset" chip for the seeded
  // corpus (a button, NOT a controlled <select> — the Playwright selectOption gotcha)
  // + the guidance-rule textarea.
  const chip = page.getByTestId("new-app-dataset-research");
  await expect(chip).toBeVisible({ timeout: 30_000 });
  await expect(chip).toContainText("research");
  await expect(page.getByTestId("new-app-rule")).toBeVisible();

  // Author: name, goal, ground on the dataset, add a guidance rule. The handle is
  // derived from the name (defaultHandle) — no handle field.
  const HANDLE = "apps/local/grounded-analyst";
  await page.getByTestId("new-app-name").fill("Grounded Analyst");
  await page.getByTestId("new-app-goal").fill("Answer questions grounded in the corpus.");
  await chip.click();
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await page.getByTestId("new-app-rule").fill("Always cite the retrieved passages.");

  await page.getByTestId("new-app-submit").click();

  // The SAVE lands (the scaffold that follows needs a served model this gateway lacks,
  // so we verify the durable envelope directly): references.datasets grounds on
  // `research` + references.rules carries the guidance note.
  await expect
    .poll(async () => (await kx.getApp(HANDLE))?.envelope !== undefined, { timeout: 30_000 })
    .toBe(true);
  const stored = await kx.getApp(HANDLE);
  const refs = (stored?.envelope as { references?: Record<string, unknown> }).references ?? {};
  expect(refs.datasets).toEqual([{ dataset_ref: "research" }]);
  const rules = refs.rules as { name: string; content_ref: string }[];
  // PR-G: every App now carries a "capabilities" rule; the authored guidance rides alongside.
  expect(rules.map((r) => r.name).sort()).toEqual(["capabilities", "guidance"]);
  const guidance = rules.find((r) => r.name === "guidance");
  expect(guidance?.content_ref ?? "").toHaveLength(64); // the guidance body → a CAS ref
  // The secret-leak invariant: the rule BODY never inlines into the envelope.
  expect(JSON.stringify(stored?.envelope)).not.toContain("Always cite");

  kx.close();
});
