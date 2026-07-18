/**
 * D213 — the New App creation flow with a kind selector. Proves you can author BOTH a
 * scheduled (functional) app and a HOSTED (experience) app from the console: the hosted
 * kind hides the workflow-planning widgets, shows the framework selector, and creates a
 * `kortecx.experience/v1` app that lands in the Hosted section. Model-free — the hosted
 * app is created by SaveApp (the page scaffold, which needs a served model, is skipped
 * gracefully; the app still runs with the framework's default page).
 */

import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("new app: a hosted (experience) app is authored and lands in the Hosted section", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();

  // From the Scheduled section, New App defaults to the scheduled kind (planning shown).
  await page.getByTestId("new-app").click();
  await expect(page.getByTestId("new-app-form")).toBeVisible();
  await expect(page.getByTestId("new-app-kind-scheduled")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("new-app-propose")).toBeVisible();

  // Switch the kind to Hosted: the workflow-planning widgets disappear; the framework
  // note + name/goal remain.
  await page.getByTestId("new-app-kind-hosted").click();
  await expect(page.getByTestId("new-app-kind-hosted")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("new-app-hosted-note")).toBeVisible();
  await expect(page.getByTestId("new-app-propose")).toHaveCount(0);
  await expect(page.getByTestId("new-app-prompt")).toHaveCount(0);

  // The framework selector is shown for hosted apps; pick a concrete framework (Svelte).
  await expect(page.getByTestId("new-app-framework")).toBeVisible();
  await page.getByTestId("new-app-framework-svelte").click();
  await expect(page.getByTestId("new-app-framework-svelte")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  // The handle field was removed — the handle is derived from the name.
  await expect(page.getByTestId("new-app-handle")).toHaveCount(0);

  // Author the hosted app (the handle is derived from the name via defaultHandle).
  const handle = "apps/local/my-site";
  await page.getByTestId("new-app-name").fill("My Site");
  await page.getByTestId("new-app-goal").fill("A simple landing page with a hero and a CTA");
  await page.getByTestId("new-app-submit").click();

  // The form closes (SaveApp succeeded; the page scaffold is skipped without a model).
  await expect(page.getByTestId("new-app-form")).toHaveCount(0, { timeout: 15_000 });

  // The new hosted app appears in the Hosted section with its live status pill + Run.
  await page.getByTestId("apps-section-hosted").click();
  await expect(page.getByTestId(`app-card-${handle}`)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId(`hosted-status-${handle}`)).toBeVisible();
  await expect(page.getByTestId(`hosted-run-${handle}`)).toBeVisible();
});
