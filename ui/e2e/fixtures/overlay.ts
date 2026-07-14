import { type Page, expect } from "@playwright/test";

/**
 * Geometry guard for the portaled section drawers/dialogs: prove the overlay renders
 * as a real full-viewport layer ABOVE the sticky navbar, not clipped behind it.
 *
 * Playwright `toBeVisible` passes on a navbar-occluded drawer (it checks the box + CSS
 * visibility, not occlusion by a higher-z-index sibling), which is why the clipping
 * regression kept slipping through. This asserts the actual fix instead:
 *   - a single `.node-drawer__scrim--overlay` sized to the viewport (mirrors the
 *     existing scrim-height proof in apps-ide.spec.ts), and
 *   - occlusion: at the navbar's own vertical mid-band, nothing resolves to `.navbar`
 *     — the fixed scrim (z 49) / panel (z 50) paint over it.
 *
 * RED pre-fix (the sticky navbar, z 10, is topmost at its band); GREEN post-fix. Note
 * we deliberately do NOT assert "drawer top >= navbar bottom": the overlay correctly
 * sits ON TOP of the navbar at y≈0 (the AppRunDrawer reference), so that check would
 * fail a correct fix. Works for both side slide-overs and the centered dialog.
 */
export async function expectOverlayAboveNavbar(page: Page, panelTestId: string): Promise<void> {
  await expect(page.getByTestId(panelTestId)).toBeVisible();

  // Exactly one overlay scrim, sized to the viewport (only one such drawer opens at a
  // time; toHaveCount(1) disambiguates the class locator + catches an accidental double-open).
  const scrim = page.locator(".node-drawer__scrim--overlay");
  await expect(scrim).toHaveCount(1);
  expect((await scrim.boundingBox())?.height ?? 0).toBeGreaterThan(100);

  // Occlusion: probe across the navbar's own vertical mid-band — none may resolve to
  // the navbar once the fixed overlay is painted over it.
  const navBox = await page.getByTestId("navbar").boundingBox();
  if (!navBox) {
    throw new Error("navbar not found");
  }
  const midY = navBox.y + navBox.height / 2;
  const xs = [0.15, 0.5, 0.85].map((f) => navBox.x + navBox.width * f);
  await expect
    .poll(() =>
      page.evaluate(
        ({ xs, midY }) => xs.every((x) => !document.elementFromPoint(x, midY)?.closest(".navbar")),
        { xs, midY },
      ),
    )
    .toBe(true);
}
