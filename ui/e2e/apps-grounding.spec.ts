/**
 * T-RUNAPP-CONTEXT-RAIL e2e: the declarative knowledge rail — a "Ground on dataset" chip
 * (over the live ListDatasets) + a guidance rule — survives the chat surface, and the saved
 * App envelope carries `references.datasets` + `references.rules`. At RunApp those resolve
 * server-side into a `retrieve@1` grant (self-grounding) + an entry-step context item.
 *
 * The rail now lives on the REVIEW panel rather than a form: the design pre-fills what it
 * asked for, and the author edits it before the App exists. Model-free — the dataset is
 * seeded via the SDK's FFI-free client-vector path (no Metal), `DeriveApp` is stubbed (the
 * one inference RPC), and we assert the SAVED envelope, not the model scaffold.
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { stubDeriveApp } from "./fixtures/grpc-stub";
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

  // The design NAMES the dataset it wants to ground on. The server has already intersected
  // that name against the caller's own non-empty datasets, so what arrives is a real chip
  // pre-pressed — not a suggestion the author has to go and find.
  await stubDeriveApp(page, {
    name: "Grounded Analyst",
    description: "Answer questions grounded in the corpus.",
    steps: [{ role: "analyst", intent: "Answer from the corpus" }],
    datasets: ["research"],
  });

  await page.getByTestId("new-app").click();
  await expect(page.getByTestId("new-app-form")).toBeVisible();
  await page
    .getByTestId("new-app-prompt")
    .fill("Answer questions grounded in the research corpus.");
  await page.getByTestId("new-app-derive").click();
  await expect(page.getByTestId("new-app-review")).toBeVisible({ timeout: 30_000 });

  // The declarative rail is on the REVIEW panel: the "Ground on dataset" chip for the seeded
  // corpus (a button, NOT a controlled <select> — the Playwright selectOption gotcha) +
  // the guidance-rule textarea.
  const chip = page.getByTestId("new-app-dataset-research");
  await expect(chip).toBeVisible({ timeout: 30_000 });
  await expect(chip).toContainText("research");
  // Pre-pressed FROM THE DESIGN — this is the derived grant landing on the rail, which is the
  // whole reason the App page arrives populated instead of empty.
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("new-app-rule")).toBeVisible();

  // The name came from the design; the handle is derived from it (defaultHandle).
  const HANDLE = "apps/local/grounded-analyst";
  await expect(page.getByTestId("new-app-name")).toHaveValue("Grounded Analyst");
  await page.getByTestId("new-app-rule").fill("Always cite the retrieved passages.");

  await page.getByTestId("new-app-approve").click();

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
