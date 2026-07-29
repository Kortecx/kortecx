/**
 * The Apps create journey — its own `/apps/create` page: derive, review, approve,
 * then a TERMINAL create result.
 *
 * Proves the HOME/CREATE split ("New App" navigates, the catalog carries no inline
 * form), that nothing is created until the design is approved, and that the journey
 * ends in an honest terminal dialog: on this model-free gateway the scaffold cannot
 * LAUNCH, so approving lands the FAILURE result — the app is saved, the dialog says
 * what happened and offers resume/discard/close — instead of silently dropping the
 * author on the App page mid-nothing.
 *
 * Model-free by stubbing the ONE inference RPC (`DeriveApp`) — the same net `nl-propose`
 * uses. Everything after the design is real: `SaveApp` hits the actual gateway, so the app
 * that lands in the Hosted section is genuinely saved.
 */

import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { stubDeriveApp } from "./fixtures/grpc-stub";
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

  await stubDeriveApp(page, {
    name: "My Site",
    description: "A simple landing page with a hero and a CTA",
    framework: "svelte",
    files: [{ path: "src/App.svelte", role: "the root component" }],
  });

  // The HOME/CREATE split: "New App" NAVIGATES to the dedicated create page (no
  // inline form on the catalog). From the Scheduled section it opens on the
  // scheduled kind, and the authoring-mode selector sits beside the kind selector.
  await page.getByTestId("new-app").click();
  await expect(page).toHaveURL(/\/apps\/create/);
  await expect(page.getByTestId("new-app-form")).toBeVisible();
  await expect(page.getByTestId("new-app-kind-scheduled")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("new-app-mode")).toBeVisible();
  await expect(page.getByTestId("new-app-prompt")).toBeVisible();

  // Switch the kind to Hosted: the mode axis disappears (a hosted app is a project by
  // construction), and the framework selector appears.
  await page.getByTestId("new-app-kind-hosted").click();
  await expect(page.getByTestId("new-app-kind-hosted")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("new-app-mode")).toHaveCount(0);
  await expect(page.getByTestId("new-app-framework")).toBeVisible();
  await page.getByTestId("new-app-framework-svelte").click();
  await expect(page.getByTestId("new-app-framework-svelte")).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  // ONE prompt is the whole input. Designing creates NOTHING — it opens the review.
  const handle = "apps/local/my-site";
  await page.getByTestId("new-app-prompt").fill("A simple landing page with a hero and a CTA");
  await page.getByTestId("new-app-derive").click();
  await expect(page.getByTestId("new-app-review")).toBeVisible({ timeout: 15_000 });

  // A HOSTED design reviews its FILE PLAN, not a workflow — the two lanes review the thing
  // each of them actually is.
  await expect(page.getByTestId("new-app-files")).toBeVisible();
  await expect(page.getByTestId("new-app-structure")).toHaveCount(0);
  await expect(page.getByTestId("new-app-file-src/App.svelte")).toBeVisible();
  // The design proposed the name; it is editable before anything exists.
  await expect(page.getByTestId("new-app-name")).toHaveValue("My Site");

  // Only NOW is the app created. On this model-free gateway the scaffold cannot
  // LAUNCH — the journey ends in the TERMINAL create-result dialog, honestly
  // reporting the failure with resume/discard/close, and the App is real regardless.
  await page.getByTestId("new-app-approve").click();
  const result = page.getByTestId("app-create-result");
  await expect(result).toBeVisible({ timeout: 15_000 });
  await expect(result).toHaveAttribute("data-outcome", "failed");
  await expect(result).toContainText("The app is saved");
  await expect(page.getByTestId("app-create-result-resume")).toBeVisible();
  await expect(page.getByTestId("app-create-result-discard")).toBeVisible();

  // Close keeps the app and returns to the catalog HOME — filed under Hosted,
  // with its live status pill + Run.
  await page.getByTestId("app-create-result-close").click();
  await expect(page).toHaveURL(/\/apps(\?|$)/, { timeout: 15_000 });
  await page.getByTestId("apps-section-hosted").click();
  await expect(page.getByTestId(`app-card-${handle}`)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId(`hosted-status-${handle}`)).toBeVisible();
  await expect(page.getByTestId(`hosted-run-${handle}`)).toBeVisible();
});

test("new app: a name that collides with an existing App is blocked, not overwritten", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();

  await stubDeriveApp(page, { name: "My Site", framework: "vite_react" });

  // Author one hosted app. (Hosted so approving needs no scaffold model — same reason as
  // above.) The design names it; the review is where the name becomes editable.
  const handle = "apps/local/my-site";
  await page.getByTestId("new-app").click();
  await page.getByTestId("new-app-kind-hosted").click();
  await page.getByTestId("new-app-prompt").fill("A simple landing page");
  await page.getByTestId("new-app-derive").click();
  await expect(page.getByTestId("new-app-review")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("new-app-approve").click();
  // Model-free ⇒ the scaffold launch fails; the terminal dialog reports it and
  // Close returns to the catalog for the second create.
  await expect(page.getByTestId("app-create-result")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("app-create-result-close").click();
  await expect(page).toHaveURL(/\/apps(\?|$)/, { timeout: 15_000 });
  await page.getByTestId("apps-section-hosted").click();
  await expect(page.getByTestId(`app-card-${handle}`)).toBeVisible({ timeout: 15_000 });

  // A second design whose proposed name derives the SAME handle. `SaveApp` upserts on the
  // handle, so without the block this silently replaces the first App's envelope and rails.
  await page.getByTestId("new-app").click();
  await page.getByTestId("new-app-kind-hosted").click();
  await page.getByTestId("new-app-prompt").fill("Something else entirely");
  await page.getByTestId("new-app-derive").click();
  await expect(page.getByTestId("new-app-review")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("new-app-name").fill("my site");

  // The collision is reported as you type, and CREATE is refused. Catching it in the review
  // is the point: the App the collision would destroy still exists, and this one does not.
  const collision = page.getByTestId("new-app-name-collision");
  await expect(collision).toBeVisible();
  await expect(collision).toContainText(handle);
  await expect(page.getByTestId("new-app-approve")).toBeDisabled();

  // Editing the name to something free clears it and re-enables create — the block must be
  // a live check, not a dead end.
  await page.getByTestId("new-app-name").fill("My Other Site");
  await expect(collision).toHaveCount(0);
  await expect(page.getByTestId("new-app-approve")).toBeEnabled();
});

test("new app: with no served model the derive REFUSES honestly, and offers the builder", async ({
  page,
}) => {
  // No `stubDeriveApp` here on purpose. A model-free gateway cannot design an app, and the
  // surface must say so rather than pretend — the App does not exist, so there is nothing to
  // half-create. The visual builder stays reachable as the model-free authoring path.
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await page.getByTestId("new-app").click();
  await page.getByTestId("new-app-prompt").fill("triage the overnight support email");
  await page.getByTestId("new-app-derive").click();

  const refusal = page
    .getByTestId("new-app-derive-rejected")
    .or(page.getByTestId("new-app-derive-error"));
  await expect(refusal.first()).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId("new-app-review")).toHaveCount(0);
  await expect(page.getByTestId("new-app-build-visual")).toBeVisible();
});
