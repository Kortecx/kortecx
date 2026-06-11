import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("Tools: the built-in manifests, an exact-hit bundle score, and the dry-run verdict", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-tools").click();
  await expect(page.getByTestId("tools-section")).toBeVisible();

  // The three OSS built-in tools render as manifest tiles — exactly those three.
  await expect(page.getByTestId("tool-manifest-grid")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("tool-tile-fs-read")).toBeVisible();
  await expect(page.getByTestId("tool-tile-fs-write")).toBeVisible();
  await expect(page.getByTestId("tool-tile-text-summarize")).toBeVisible();
  await expect(page.locator('[data-testid^="tool-tile-"]')).toHaveCount(3);

  // Compose: an intent whose words exact-hit fs-read's curated keywords, plus the
  // fs-read chip. The intent input is CONTROLLED — click + pressSequentially, never
  // a bulk fill() (the recorded React-controlled-input e2e gotcha).
  const intent = page.getByTestId("bundle-intent");
  await intent.click();
  await intent.pressSequentially("read a file from disk");

  await page.getByTestId("tool-chip-fs-read").click();
  await expect(page.getByTestId("tool-chip-fs-read")).toHaveAttribute("aria-pressed", "true");

  await page.getByTestId("bundle-score").click();

  // The ladder: fs-read ranks 10000bp = the Exact rung.
  const fsReadRow = page.getByTestId("score-row-fs-read");
  await expect(fsReadRow).toBeVisible({ timeout: 30_000 });
  await expect(fsReadRow).toContainText("10000");
  await expect(fsReadRow).toContainText("Exact");

  // The bundle fingerprint is server-derived 32B blake3, shown as 64-char hex.
  await expect(page.getByTestId("bundle-fingerprint")).toHaveText(/^[0-9a-f]{64}$/);

  // No inference on this serve → the lowering dry-run reports no live model
  // (advisory verdict — SN-8: display-only, never authorization).
  await expect(page.getByTestId("verdict-badge")).toContainText("No live model");
  await expect(page.getByText("Advisory only — scores never authorize.")).toBeVisible();
});
