import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("the model picker hides on a model-less serve (honest empty — no fake knob)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();

  // The FFI-free test serve answers ListModels with an EMPTY list — the picker
  // must not render (and nothing crashes).
  await expect(page.getByTestId("model-picker")).toHaveCount(0);
  // The chat surface stays fully usable.
  await expect(page.getByLabel(/^message$/i)).toBeEnabled();
  await expect(page.getByTestId("attach-btn")).toBeVisible();
});
