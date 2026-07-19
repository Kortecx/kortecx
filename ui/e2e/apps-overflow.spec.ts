/**
 * Bug B — a long App name/handle must NOT overflow its card, the table, or the
 * page shell. A long name sanitizes to a ~139-char handle; without the containment CSS the
 * unbreakable mono handle blows its grid column and spills across the section boundary.
 *
 * Geometry net (mirrors the overlay geometry-proof style): assert no horizontal overflow of
 * the catalog, the table, and the shell, and that the handle truncates (ellipsis + a title=
 * carrying the full value). RED before the app.css `.card-grid__card { min-width: 0 }` +
 * `.card-grid__handle` truncation; GREEN after. Model-free (a trivial pure blueprint).
 */

import { KxClient } from "@kortecx/sdk/node";
import { type Locator, expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

const LONG_NAME =
  "Recipe Card Random Shuffle Dashboard With A Very Long Name To Test Handle Overflow In The Console";
const HANDLE = `apps/local/${LONG_NAME.toLowerCase()
  .replace(/[^a-z0-9._-]/g, "-")
  .replace(/^[.-]+|[.-]+$/g, "")
  .slice(0, 128)}`;

/** True iff the element has no horizontal overflow (content fits its box, ±1px rounding). */
async function noHorizontalOverflow(el: Locator): Promise<boolean> {
  return el.evaluate((n) => n.scrollWidth <= n.clientWidth + 1);
}

test("apps: a long App name/handle does not overflow its card, table, or the shell", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  const seed = new KxClient(gw.endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: LONG_NAME,
      blueprint: { seed: 0, steps: [{ kind: "pure", params: { note: "x" } }] },
    },
    { handle: HANDLE },
  );
  seed.close();

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-catalog")).toBeVisible();

  // The handle chip is present, carries the full handle on title=, and truncates (ellipsis) so
  // the full value stays discoverable on hover without spilling out of the card.
  const handle = page.locator(".card-grid__handle").first();
  await expect(handle).toHaveAttribute("title", /^apps\/local\//);
  expect(await handle.evaluate((n) => getComputedStyle(n).textOverflow)).toBe("ellipsis");

  // Box (card) view — no horizontal overflow of the catalog or the page shell.
  expect(await noHorizontalOverflow(page.getByTestId("apps-catalog"))).toBe(true);
  expect(await noHorizontalOverflow(page.locator(".shell__main"))).toBe(true);

  // Table view — the row handle must not widen the table past its container either.
  await page.getByTestId("apps-view-table").click();
  await expect(page.getByTestId("apps-table")).toBeVisible();
  expect(await noHorizontalOverflow(page.getByTestId("apps-table"))).toBe(true);
  expect(await noHorizontalOverflow(page.locator(".shell__main"))).toBe(true);
});
