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
  // The PR-B group label is visible when expanded, gone on the rail.
  const group = page.getByTestId("nav-group-data");
  await expect(sidebar).toHaveAttribute("data-collapsed", "false");
  await expect(item).toContainText("Blueprints");
  await expect(group).toContainText("Data");

  await page.getByTestId("sidebar-toggle").click();
  await expect(sidebar).toHaveAttribute("data-collapsed", "true");
  await expect(item).not.toContainText("Blueprints"); // icon-only rail
  await expect(group).not.toContainText("Data"); // group labels drop on the rail

  // Connect is a login gate (D137): a reload drops the in-memory token, so the
  // shell is GONE until we reconnect — and the persisted collapse then survives.
  await page.reload();
  await expect(page.getByTestId("app-gate")).toBeVisible();
  await expect(page.getByTestId("sidebar")).toHaveCount(0);
  await connectConsole(page, gw);
  await expect(page.getByTestId("sidebar")).toHaveAttribute("data-collapsed", "true");
});
