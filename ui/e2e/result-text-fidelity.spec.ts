import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

/**
 * D142.2 content-resolution: every run-output surface shows the RESOLVED result
 * TEXT as the headline (with the digest demoted to a copy chip), never a bare
 * hash. The model-free echo recipe commits a fully-printable demo result
 * ("kx demo result for mote <hex>\n"), so the resolved text is a stable,
 * assertable string across the table, DAG node, artifact list, and event feed —
 * in BOTH themes.
 */

const DEMO_TEXT = "kx demo result for mote";
let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("run outputs resolve to TEXT across table, DAG, artifacts + feed (both themes)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  // WS bridge so the run-scoped Activity feed tails real committed deltas.
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "fidelity" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBeGreaterThan(0);

  // --- DAG node: the resolved text is on the node (a glimpse of the output). ---
  await expect(page.locator(".dag-node__result").first()).toContainText(DEMO_TEXT, {
    timeout: 30_000,
  });

  // --- Table: the Result column is the resolved text headline + a digest chip. ---
  await page.getByTestId("run-tab-table").click();
  const preview = page.getByTestId("result-preview").first();
  await expect(preview).toHaveAttribute("data-state", "text", { timeout: 30_000 });
  await expect(preview).toContainText(DEMO_TEXT);
  // the digest rides as the SECONDARY affordance (present, not the headline)
  await expect(preview.getByTestId("digest-chip")).toBeVisible();

  // --- Artifacts: each list row leads with the resolved text. ---
  await page.getByTestId("run-tab-artifacts").click();
  await expect(page.getByTestId("artifact-gallery")).toBeVisible({ timeout: 30_000 });
  await expect(page.locator(".artifact-list__row .result-preview").first()).toContainText(
    DEMO_TEXT,
    { timeout: 30_000 },
  );

  // --- Activity feed: the committed event row shows the resolved text. ---
  await page.getByTestId("run-tab-activity").click();
  await expect(page.getByTestId("run-activity-tab")).toBeVisible();
  const feedRow = page.getByTestId("event-row").filter({ hasText: DEMO_TEXT }).first();
  await expect(feedRow).toBeVisible({ timeout: 30_000 });

  // --- Both themes: the resolution is theme-agnostic (tokens only). Flip to dark
  //     and re-confirm the table headline still resolves to text (not a hash). ---
  await page.getByTestId("nav-settings").click();
  await expect(page.getByTestId("settings-section")).toBeVisible();
  await page.getByTestId("theme-chip-dark").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

  await page.getByTestId("nav-runs").click();
  await page.getByTestId("run-open").first().click();
  await page.getByTestId("run-tab-table").click();
  const darkPreview = page.getByTestId("result-preview").first();
  await expect(darkPreview).toHaveAttribute("data-state", "text", { timeout: 30_000 });
  await expect(darkPreview).toContainText(DEMO_TEXT);
});
