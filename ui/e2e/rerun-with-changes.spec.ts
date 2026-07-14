import { type Page, expect, test } from "@playwright/test";
import { connectConsole, gotoRunHistory, runRecipe } from "./fixtures/connect";
import { expectOverlayAboveNavbar } from "./fixtures/overlay";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

/** Open the run-detail drawer for the first run, then the "Re-run with changes" drawer. */
async function openRerun(page: Page): Promise<void> {
  await gotoRunHistory(page);
  await expect(page.getByTestId("run-list")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("run-open").first().click();
  await expect(page.getByTestId("run-detail-drawer")).toBeVisible();
  await expectOverlayAboveNavbar(page, "run-detail-drawer");
  await page.getByTestId("run-rerun-changes").click();
  await expect(page.getByTestId("rerun-drawer")).toBeVisible();
  await expectOverlayAboveNavbar(page, "rerun-drawer");
}

/** Submit the re-run form, dismissing the conservative confirm if it appears
 *  (the prior run's projection may still be loading for a fresh drawer). */
async function submitRerun(page: Page): Promise<void> {
  await page.getByRole("button", { name: /run blueprint/i }).click();
  const confirmFire = page.getByTestId("rerun-confirm-fire");
  if (await confirmFire.isVisible().catch(() => false)) {
    await confirmFire.click();
  }
}

test("rerun: unchanged args dedup to the existing result (no-change banner)", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await runRecipe(page, { fields: { topic: "alpha" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  await openRerun(page);
  // The form is pre-filled with the original args.
  await expect(page.getByTestId("field-topic")).toHaveValue("alpha");
  // Submit UNCHANGED → the kernel dedups back to the same terminal mote.
  await submitRerun(page);
  await expect(page.getByTestId("rerun-no-change")).toBeVisible({ timeout: 30_000 });
});

test("rerun: a changed arg fires a fresh run and routes to its DAG", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await runRecipe(page, { fields: { topic: "alpha" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  await openRerun(page);
  const topic = page.getByTestId("field-topic");
  await topic.click();
  await topic.fill("");
  await topic.pressSequentially("beta");
  await submitRerun(page);
  // A changed arg ⇒ a new terminal ⇒ route to the run DAG (no no-change banner).
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("rerun-no-change")).toHaveCount(0);
});

test("rerun: a durable run (no local history) recovers its args via GetRunInputs", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await runRecipe(page, { fields: { topic: "durable-alpha" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // Wipe THIS browser's history + query cache (the fresh-session recovery case):
  // the run now comes back only from the durable journal (the `journal` badge).
  await page.evaluate(() => window.localStorage.clear());
  await page.reload();
  await connectConsole(page, gw);
  await gotoRunHistory(page);
  await expect(page.getByTestId("run-list")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("run-open").first().click();
  await expect(page.getByTestId("run-detail-drawer")).toBeVisible();
  await page.getByTestId("run-rerun-changes").click();
  await expect(page.getByTestId("rerun-drawer")).toBeVisible();
  // The args were recovered from the off-journal sidecar via GetRunInputs.
  await expect(page.getByTestId("field-topic")).toHaveValue("durable-alpha", { timeout: 30_000 });
});
