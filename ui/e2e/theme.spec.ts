import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("the theme toggle switches palettes and persists across reload (pre-paint)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  const html = page.locator("html");
  // Playwright's chromium defaults to a light OS preference → system resolves light.
  await expect(html).toHaveAttribute("data-theme", "light");

  await page.getByTestId("nav-settings").click();
  await expect(page.getByTestId("settings-section")).toBeVisible();
  await page.getByTestId("theme-chip-dark").click();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(page.getByTestId("theme-resolved")).toContainText("kortecx dark");
  // the body actually paints the terminal-dark page bg (#09090b)
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).toBe("rgb(9, 9, 11)");

  // Reload drops the in-memory token (login gate) but NOT the theme: the
  // index.html pre-paint script stamps dark before first paint.
  await page.reload();
  await expect(page.getByTestId("app-gate")).toBeVisible();
  await expect(html).toHaveAttribute("data-theme", "dark");

  // Reconnect and smoke a data section in dark (DAG/catalog tokens re-resolve).
  await connectConsole(page, gw);
  await expect(html).toHaveAttribute("data-theme", "dark");
  await gotoViaPalette(page, "recipes");
  await expect(page.getByTestId("recipe-catalog")).toBeVisible({ timeout: 30_000 });

  // The Apps catalog renders under the dark palette (card + chip tones re-resolve;
  // the contrast lock covers the text tiers).
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible({ timeout: 15_000 });

  // Back to system → light again (and the chip state follows).
  await page.getByTestId("nav-settings").click();
  await page.getByTestId("theme-chip-system").click();
  await expect(html).toHaveAttribute("data-theme", "light");
  await expect(page.getByTestId("theme-chip-system")).toHaveAttribute("aria-pressed", "true");
});

test.describe("OS dark preference", () => {
  test.use({ colorScheme: "dark" });

  test("'system' follows prefers-color-scheme: dark from the very first paint", async ({
    page,
  }) => {
    gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
    await page.goto("/connect");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

    // An explicit light choice overrides the OS signal.
    await connectConsole(page, gw);
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("theme-chip-light").click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });
});
