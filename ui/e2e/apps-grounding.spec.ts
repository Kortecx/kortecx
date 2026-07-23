/**
 * T-RUNAPP-CONTEXT-RAIL e2e: grounding is a per-NODE capability now. A step names the dataset
 * it grounds on in the step drawer; the saved envelope carries that name as a per-step
 * `datasets` binding AND declares it in `references.datasets`. At RunApp the bound step gets a
 * `retrieve@1` grant + a grounding steer.
 *
 * The dataset chip lives on the node — there is no rail beside the canvas. Model-free: the
 * corpus is seeded via the SDK's FFI-free client-vector path (no Metal), `DeriveApp` is stubbed
 * (the one inference RPC), and we assert the SAVED envelope, not the model scaffold.
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

test("New App: a grounded App binds the dataset to the NODE, and the envelope declares it", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // Seed a corpus via the FFI-free client-vector path (no model / no Metal) so grounding has
  // a non-empty dataset to offer.
  const kx = new KxClient(gw.endpoint);
  await kx.ingestDocuments("research", [
    { content: new TextEncoder().encode("alpha"), embedding: [1, 0, 0, 0.1] },
    { content: new TextEncoder().encode("bravo"), embedding: [0, 1, 0, 0.1] },
  ]);

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();

  // The design binds the dataset to the STEP that grounds on it. The server has already
  // intersected that name against the caller's own non-empty datasets, so what arrives is a
  // real binding on a node — not a suggestion the author has to go and find.
  await stubDeriveApp(page, {
    name: "Grounded Analyst",
    description: "Answer questions grounded in the corpus.",
    steps: [{ role: "analyst", intent: "Answer from the corpus", datasets: ["research"] }],
  });

  await page.getByTestId("new-app").click();
  await expect(page.getByTestId("new-app-form")).toBeVisible();
  await page
    .getByTestId("new-app-prompt")
    .fill("Answer questions grounded in the research corpus.");
  await page.getByTestId("new-app-derive").click();
  await expect(page.getByTestId("new-app-review")).toBeVisible({ timeout: 30_000 });

  // There is NO rail beside the canvas — grounding is on the node, so opening the step is how
  // you see (and edit) what it grounds on.
  await expect(page.getByTestId("new-app-datasets")).toHaveCount(0);
  await page.getByTestId("builder-node").first().click();
  await expect(page.getByTestId("step-config-drawer")).toBeVisible();
  // The design's grounding lands on the node, pre-pressed — the derived binding, which is why
  // the App page arrives populated instead of empty.
  const chip = page.getByTestId("step-config-datasets-research");
  await expect(chip).toBeVisible({ timeout: 30_000 });
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");

  // The name came from the design; the handle is derived from it (defaultHandle).
  const HANDLE = "apps/local/grounded-analyst";
  await expect(page.getByTestId("new-app-name")).toHaveValue("Grounded Analyst");

  await page.getByTestId("new-app-approve").click();

  // The SAVE lands (the scaffold that follows needs a served model this gateway lacks, so we
  // verify the durable envelope directly).
  await expect
    .poll(async () => (await kx.getApp(HANDLE))?.envelope !== undefined, { timeout: 30_000 })
    .toBe(true);
  const stored = await kx.getApp(HANDLE);
  // references.datasets DECLARES the dataset (what must be registered)...
  const refs = (stored?.envelope as { references?: Record<string, unknown> }).references ?? {};
  expect(refs.datasets).toEqual([{ dataset_ref: "research" }]);
  // ...and the blueprint step BINDS it (which node grounds on it) — the per-node truth.
  const bp = (
    (stored as { envelope: unknown }).envelope as {
      blueprint: { steps: { datasets?: string[] }[] };
    }
  ).blueprint;
  expect(bp.steps[0]?.datasets).toEqual(["research"]);
  // Every App still carries the capabilities rule (co-shipped since PR-G).
  const rules = (refs.rules as { name: string }[]) ?? [];
  expect(rules.map((r) => r.name)).toContain("capabilities");

  kx.close();
});
