import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("the model control is VISIBLE with an honest empty state on a model-less serve", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();

  // The FFI-free test serve answers ListModels with an EMPTY list — the control
  // stays VISIBLE (PR-1.1 review feedback) as an honest, disabled empty state;
  // the SELECT (a knob with nothing to pick) does not render.
  await expect(page.getByTestId("model-picker-empty")).toBeVisible();
  await expect(page.getByTestId("model-picker-empty")).toContainText("none on this serve");
  await expect(page.getByTestId("model-picker-select")).toHaveCount(0);
  // The chat surface stays fully usable.
  await expect(page.getByTestId("attach-btn")).toBeVisible();
});
