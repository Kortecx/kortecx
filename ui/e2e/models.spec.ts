import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("Models: honest-empty + disabled-Cloud on a model-less serve (both themes)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Models is a read-only Tools-group section (PR-C2).
  await page.getByTestId("nav-models").click();
  await expect(page.getByTestId("models-section")).toBeVisible();

  // The FFI-free test serve answers ListModels with an EMPTY list → an honest empty
  // state, never a fabricated row or a spinner that never resolves (GR15).
  await expect(page.getByText(/no models on this serve/i)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("model-card")).toHaveCount(0);

  // Connecting a vendor is Cloud (disabled, never faked). Model Control v2: this test
  // serve runs with downloads OFF (no KX_SERVE_ALLOW_MODEL_PULL), so the Pull panel
  // renders its honest-disabled state with the reason — never a faked control.
  const connect = page.getByTestId("models-cloud-connect");
  await expect(connect).toBeVisible();
  await expect(connect).toHaveAttribute("aria-disabled", "true");
  const pullDisabled = page.getByTestId("models-pull-disabled");
  await expect(pullDisabled).toHaveAttribute("aria-disabled", "true");
  await expect(pullDisabled).toContainText("KX_SERVE_ALLOW_MODEL_PULL");

  // BOTH THEMES (D142.1 / GR13): the section renders under the dark palette.
  await page.getByTestId("theme-toggle").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.getByTestId("models-section")).toBeVisible();
  await expect(page.getByText(/no models on this serve/i)).toBeVisible();
});
