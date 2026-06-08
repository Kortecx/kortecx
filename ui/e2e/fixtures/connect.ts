import { type Page, expect } from "@playwright/test";
import type { Gateway } from "./serve";

/**
 * Drive the connect screen to a connected console. Optionally set the advanced WS
 * endpoint (for the Activity live tail). Resolves once the Activity dashboard shows.
 */
export async function connectConsole(
  page: Page,
  gw: Gateway,
  opts: { wsEndpoint?: string } = {},
): Promise<void> {
  await page.goto("/connect");
  await page.getByLabel(/gateway endpoint/i).fill(gw.endpoint);
  if (opts.wsEndpoint) {
    await page.getByText(/^advanced$/i).click();
    await page.getByLabel(/ws bridge endpoint/i).fill(opts.wsEndpoint);
  }
  await page.getByRole("button", { name: /^connect$/i }).click();
  await expect(page.getByTestId("activity-panel")).toBeVisible({ timeout: 30_000 });
}
