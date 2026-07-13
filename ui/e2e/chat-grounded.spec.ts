import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

/**
 * New Chat is READ-ONLY, RAG-grounded — but the standalone dataset grounding
 * BAR is gone (dataset-grounded RAG relocates to Apps, Principle-3). Grounding is now
 * the header "Context" attach button + the attached-file chips. This spec asserts the
 * clean read-only SHAPE in BOTH themes (the GR13 both-theme gate) over a real gateway.
 */

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("New Chat is a clean read-only chat (no dataset bar, header Context attach) in BOTH themes", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();

  // The standalone grounding bar + its dataset picker are gone; there is no Agent toggle.
  await expect(page.getByTestId("chat-grounding")).toHaveCount(0);
  await expect(page.getByTestId("dataset-picker")).toHaveCount(0);
  await expect(page.getByTestId("chat-mode")).toHaveCount(0);
  // The header "Context" attach control is the first-class read-only grounding affordance.
  await expect(page.getByTestId("chat-grounding-add")).toBeVisible();

  // Both themes (D142 / GR13): the panel + the Context control stay legible in light AND dark.
  for (const theme of ["light", "dark"] as const) {
    const current = await page.locator("html").getAttribute("data-theme");
    if (current !== theme) {
      await page.getByTestId("theme-toggle").click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.getByTestId("chat-panel")).toBeVisible();
    await expect(page.getByTestId("chat-grounding-add")).toBeVisible();
  }
});
