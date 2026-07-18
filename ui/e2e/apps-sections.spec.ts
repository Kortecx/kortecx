/**
 * D213 — the two-section Apps page (Scheduled vs Hosted) + the box/table view toggle.
 * Seeds one functional (scheduled) app and one experience (hosted) app, then proves the
 * section switch partitions the catalog by lane, the hosted section carries a live status
 * pill + Run control, and the box/table icon toggle swaps the layout (round-tripping the
 * `?view=` param). Model-free (both apps have trivial/no blueprints).
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

const SCHEDULED = "apps/local/digest";
const HOSTED = "apps/local/landing";

test("apps: Scheduled/Hosted sections partition the catalog; box/table toggle swaps layout", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  const seed = new KxClient(gw.endpoint);
  // A functional (scheduled) app.
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Daily Digest",
      blueprint: { seed: 0, steps: [{ kind: "pure", params: { note: "demo" } }] },
    },
    { handle: SCHEDULED },
  );
  // A hosted (experience) app — no blueprint, a hosted config + a project branch.
  await seed.saveApp(
    {
      schema: "kortecx.experience/v1",
      name: "Landing Page",
      hosted: { framework: "vite_react" },
      branch_handle: HOSTED,
    },
    { handle: HOSTED },
  );
  seed.close();

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();

  // Both section tabs + Import + New App are present.
  await expect(page.getByTestId("apps-section-scheduled")).toBeVisible();
  await expect(page.getByTestId("apps-section-hosted")).toBeVisible();
  await expect(page.getByTestId("import-app")).toBeVisible();
  await expect(page.getByTestId("new-app")).toBeVisible();

  // Default = Scheduled: the functional app shows, the hosted one does NOT.
  await expect(page.getByTestId(`app-card-${SCHEDULED}`)).toBeVisible();
  await expect(page.getByTestId(`app-card-${HOSTED}`)).toHaveCount(0);

  // Switch to Hosted: the hosted app shows with a live status pill + Run control; the
  // scheduled one is gone. The URL carries ?section=hosted.
  await page.getByTestId("apps-section-hosted").click();
  await expect(page).toHaveURL(/section=hosted/);
  await expect(page.getByTestId(`app-card-${HOSTED}`)).toBeVisible();
  await expect(page.getByTestId(`app-card-${SCHEDULED}`)).toHaveCount(0);
  await expect(page.getByTestId(`hosted-status-${HOSTED}`)).toBeVisible();
  await expect(page.getByTestId(`hosted-run-${HOSTED}`)).toBeVisible();

  // The box/table toggle swaps the card grid for a table (round-trips ?view=table).
  await expect(page.getByTestId("apps-catalog")).toBeVisible();
  await page.getByTestId("apps-view-table").click();
  await expect(page).toHaveURL(/view=table/);
  await expect(page.getByTestId("apps-table")).toBeVisible();
  await expect(page.getByTestId("apps-catalog")).toHaveCount(0);
  await expect(page.getByTestId(`app-row-${HOSTED}`)).toBeVisible();

  // Back to box.
  await page.getByTestId("apps-view-box").click();
  await expect(page.getByTestId("apps-catalog")).toBeVisible();
  await expect(page.getByTestId("apps-table")).toHaveCount(0);
});
