/**
 * The console driven against a REAL MODEL — the seam nothing had ever used.
 *
 * Every other console spec is model-free by construction: `spawnGateway` forces
 * `KX_SERVE_OLLAMA=off` so the suite is deterministic and matches CI, which has no
 * Ollama daemon. That left the two RPCs the whole "describe it and get one" story
 * rests on — `DeriveApp` and `ProposeWorkflow` — never once exercised from the
 * browser. The `model: true` opt-in existed; these are the specs that use it.
 *
 * ⚠ OPT-IN, and it must stay that way. `test.skip` below is not politeness: without
 * it CI would spawn a gateway expecting a model that is not there, and the failure
 * would look like a console defect.
 *
 * Run it:
 * ```
 * cargo build --release -p kx-cli --bin kx \
 *   --features console,serve-engine,hnsw,hosted-apps,observability
 * KX_NL_LIVE=1 KX_MODEL_BIN=$PWD/../target/release/kx \
 *   npx playwright test e2e/nl-authoring-live.spec.ts
 * ```
 *
 * ⚠ `KX_MODEL_BIN` must be a `serve-engine` build. The fixture refuses to fall back
 * to the model-less e2e binary, because there `DeriveApp` is not merely modelless —
 * it is not compiled in at all and answers `unimplemented`, which would read as a
 * console bug rather than a build mistake.
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

// Opt-in only. CI has no Ollama daemon and no serve-engine binary.
test.skip(
  !process.env.KX_NL_LIVE,
  "live NL authoring: set KX_NL_LIVE=1 with KX_MODEL_BIN pointing at a serve-engine kx",
);

// A 12B model plans for a while; the default 120s is not enough for derive → approve.
test.setTimeout(360_000);

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

/**
 * DERIVE, unstubbed, end to end: a description becomes a real design in the browser,
 * and approving it lands a real envelope the node client reads back out-of-band.
 *
 * The model-free sibling (`nl-propose.spec.ts`) stubs `DeriveApp` because a
 * model-free gateway would honestly reject. Here nothing is stubbed — which means a
 * pass is evidence the served model produced an admissible design, not that a canned
 * one rendered.
 */
test("apps: a described app is DERIVED by a live model, approved, and lands", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN, model: true });
  const handle = `apps/local/live-derive-${Date.now()}`;
  const name = handle.split("/").pop() ?? "live-derive";

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await page.getByTestId("new-app").click();

  await page
    .getByTestId("new-app-prompt")
    .fill(
      "Every morning, collect the issues filed on our repo overnight, group them by " +
        "theme, and write a short digest.",
    );
  await page.getByTestId("new-app-derive").click();

  // The design renders — or the model honestly refused, which is a real outcome on a
  // 12B and is reported rather than retried into a pass.
  const structure = page.getByTestId("new-app-structure");
  const rejected = page.getByTestId("new-app-derive-rejected");
  await expect(structure.or(rejected)).toBeVisible({ timeout: 300_000 });
  if (await rejected.isVisible()) {
    const reason = await rejected.textContent();
    test.info().annotations.push({ type: "live-refusal", description: reason ?? "" });
    // A refusal must still SAY something — a blank refusal is the failure mode that
    // leaves an author with nothing to act on.
    expect(reason?.trim()).not.toBe("");
    test.skip(true, `the live model refused this design: ${reason?.trim()}`);
    return;
  }

  // A real design, not an empty shell: the derive lowered into canvas nodes.
  await expect(page.getByTestId("builder-node").first()).toBeVisible({ timeout: 60_000 });
  const steps = await page.getByTestId("builder-node").count();
  expect(steps, "the derived design carries at least one step").toBeGreaterThan(0);

  // Name it, then approve. `SaveApp` is NOT stubbed anywhere in this suite by design,
  // so this writes a real envelope.
  const nameField = page.getByTestId("new-app-name");
  if (await nameField.isVisible()) {
    await nameField.fill(name);
  }
  await expect(page.getByTestId("new-app-approve")).toBeEnabled({ timeout: 30_000 });
  await page.getByTestId("new-app-approve").click();

  // THE ASSERTION, taken out-of-band through the node client rather than from the
  // screen: the App genuinely landed on the gateway.
  const kx = new KxClient(gw.endpoint);
  try {
    await expect
      .poll(
        async () => (await kx.listApps()).some((a) => a.name === name || a.handle.includes(name)),
        {
          timeout: 120_000,
        },
      )
      .toBe(true);
  } finally {
    kx.close();
  }
});

/**
 * PROPOSE, unstubbed: a goal becomes a real multi-step plan and lowers onto the
 * canvas.
 *
 * The model-free sibling proves the lowering with a canned plan. This proves the
 * other half — that a served model, driven from the browser, returns a plan the
 * gateway's own compile gate accepts.
 */
test("builder: a live model PROPOSES a plan and it lowers onto the canvas", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN, model: true });

  await connectConsole(page, gw);
  await gotoViaPalette(page, "recipes");
  await page.getByTestId("new-blueprint").click();
  await expect(page.getByTestId("builder-canvas")).toBeVisible({ timeout: 30_000 });

  const nodes = page.getByTestId("builder-node");
  const before = await nodes.count();

  await page.getByTestId("builder-propose").click();
  await page
    .getByTestId("builder-propose-goal")
    .fill("Research the top 3 durable-execution engines and write a short comparison.");
  await page.getByTestId("builder-propose-submit").click();

  const proposed = page.getByTestId("builder-propose-steps");
  const refused = page.getByTestId("builder-propose-reject");
  await expect(proposed.or(refused)).toBeVisible({ timeout: 300_000 });
  if (await refused.isVisible()) {
    const reason = await refused.textContent();
    expect(reason?.trim()).not.toBe("");
    test.skip(true, `the live model refused this goal: ${reason?.trim()}`);
    return;
  }

  const count = await page.getByTestId("builder-propose-step").count();
  expect(count, "a live proposal is a MULTI-step plan").toBeGreaterThanOrEqual(2);

  await page.getByTestId("builder-propose-apply").click();
  await expect(page.getByTestId("builder-propose-panel")).toHaveCount(0);
  await expect(nodes).toHaveCount(before + count);
  await expect(page.getByTestId("builder-node-needs-config")).toHaveCount(0);
});
