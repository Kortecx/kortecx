/**
 * POC-5d (Authoring · structure): editing a saved App's executable structure in the
 * visual builder end-to-end in a real browser (model-free). Seeds an App through the
 * node SDK, then drives: Lineage → "Edit structure" → the builder seeded from the App's
 * blueprint → add a step → "Save to App" → the new structure PERSISTS (the Lineage
 * diagram re-renders with the added step). Also covers "Save as App" (the builder mints
 * a brand-new App) reached from the New-App form's "build visually" link, and the
 * builder app-edit surfaces in BOTH themes. SaveApp is model-free; the real agentic
 * proof that the edited structure changes execution runs on the Gemma serve (Rule 41).
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

const HANDLE = "apps/local/struct-demo";

/** Seed a single-agent (model-step) App + its project branch. A model step with no
 *  model_id is round-trippable (not exec) and served-model-bindable, so the editor
 *  opens and Save-to-App is allowed even with no model served. */
async function seedApp(endpoint: string): Promise<void> {
  const seed = new KxClient(endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Struct Demo",
      blueprint: { seed: 0, steps: [{ kind: "model", prompt: "summarize the input" }] },
    },
    { handle: HANDLE },
  );
  await seed.createBranch(HANDLE);
  seed.close();
}

/** Open the App's Lineage pane via the Apps catalog → card menu → Open project. */
async function openLineage(page: import("@playwright/test").Page): Promise<void> {
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId(`app-menu-${HANDLE}`).click();
  await page.getByTestId(`app-open-${HANDLE}`).click();
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("app-tab-lineage").click();
  await expect(page.getByTestId("app-lineage")).toBeVisible();
}

test("Edit structure → add a step → Save to App persists the new structure", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await seedApp(gw.endpoint);
  await connectConsole(page, gw);
  await openLineage(page);

  // The seeded App is round-trippable + unlocked ⇒ "Edit structure" is offered and the
  // diagram shows exactly the one seeded step.
  await expect(page.getByTestId("app-lineage-diagram")).toHaveAttribute("data-steps", "1");
  await expect(page.getByTestId("lineage-edit-structure")).toBeVisible();

  // Open the builder seeded from this App (the ?app=<handle> route).
  await page.getByTestId("lineage-edit-structure").click();
  await expect(page).toHaveURL(/\/blueprints\/new\?app=/);
  await expect(page.getByTestId("blueprint-builder")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("builder-canvas")).toBeVisible();
  // Seeded from the App's blueprint: one node (the agent step). And this is app-edit
  // mode — the terminal is "Save to App", NOT "Build & run".
  await expect(page.getByTestId("builder-node")).toHaveCount(1);
  await expect(page.getByTestId("builder-save-app")).toBeVisible();
  await expect(page.getByTestId("builder-submit")).toHaveCount(0);

  // Edit the structure: add a Pure step (closing the config drawer it opens so the
  // toolbar is clickable — the shared-scrim reachability pattern).
  await page.getByTestId("builder-add-pure").click();
  await expect(page.getByTestId("builder-node")).toHaveCount(2);
  await page.keyboard.press("Escape");

  // Save to App → back on the App's Lineage, the diagram now shows the added step
  // (the structure edit round-tripped through SaveApp + a re-fetch).
  await page.getByTestId("builder-save-app").click();
  await expect(page).toHaveURL(/\/apps\/.*[?&]tab=lineage/, { timeout: 30_000 });
  await expect(page.getByTestId("app-lineage-diagram")).toHaveAttribute("data-steps", "2", {
    timeout: 30_000,
  });
});

test("the builder app-edit surfaces render in both themes", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await seedApp(gw.endpoint);
  await connectConsole(page, gw);
  await openLineage(page);
  await page.getByTestId("lineage-edit-structure").click();
  await expect(page.getByTestId("blueprint-builder")).toBeVisible({ timeout: 30_000 });

  for (const theme of ["light", "dark"] as const) {
    const current = await page.locator("html").getAttribute("data-theme");
    if (current !== theme) {
      await page.getByTestId("theme-toggle").click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    // The editable canvas, the seeded node, and the Save-to-App terminal all render.
    await expect(page.getByTestId("builder-canvas")).toBeVisible();
    await expect(page.getByTestId("builder-node")).toHaveCount(1);
    await expect(page.getByTestId("builder-save-app")).toBeVisible();
  }
});

test("Save as App: the builder mints a new App (reached from New App → build visually)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // New-App form offers a "build the structure yourself" link into the visual builder.
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId("new-app").click();
  await expect(page.getByTestId("new-app-form")).toBeVisible();
  await page.getByTestId("new-app-build-visual").click();

  // The builder in workflow mode: one starter agent, both a Build & run and a Save as
  // App terminal. With no model served, Save as App is still allowed (allowEmptyModel).
  await expect(page.getByTestId("blueprint-builder")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("builder-submit")).toBeVisible();
  await expect(page.getByTestId("builder-save-as-app")).toBeEnabled();

  // Save as App → name dialog → submit → land on the new App's page.
  await page.getByTestId("builder-save-as-app").click();
  await expect(page.getByTestId("builder-save-as-dialog")).toBeVisible();
  await page.getByTestId("builder-save-as-name").fill("Structy New App");
  await page.getByTestId("builder-save-as-submit").click();
  await expect(page).toHaveURL(/\/apps\//, { timeout: 30_000 });
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });
});
