import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("the sidebar collapses to an icon rail and the choice persists across reload", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  const sidebar = page.getByTestId("sidebar");
  // Assert on a nav item's text (avoids colliding with the page's <h1> headings).
  const item = page.getByTestId("nav-recipes");
  await expect(sidebar).toHaveAttribute("data-collapsed", "false");
  await expect(item).toContainText("Recipes");

  await page.getByTestId("sidebar-toggle").click();
  await expect(sidebar).toHaveAttribute("data-collapsed", "true");
  await expect(item).not.toContainText("Recipes"); // icon-only rail

  // The shell renders even when disconnected, so the persisted collapse survives a reload.
  await page.reload();
  await expect(page.getByTestId("sidebar")).toHaveAttribute("data-collapsed", "true");
});
