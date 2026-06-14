import { expect, test } from "@playwright/test";
import { typeMessage } from "./fixtures/chat";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("chat round-trips against the model-free echo recipe (Invoke→poll→GetContent)", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();

  // PR-B (GR15): on this model-free serve chat proactively shows the honest
  // "no model — connect one" notice (instead of silently echoing).
  await expect(page.getByTestId("degrade-notice")).toBeVisible();

  // Point chat at the deterministic, model-free echo recipe.
  await page.getByTestId("chat-settings").locator("summary").click();
  await page.getByTestId("echo-preset").click();

  // Picking echo is a DELIBERATE model-free choice — the notice is dismissed
  // (resolveChatBacking honors echo verbatim; it no longer masks a gap).
  await expect(page.getByTestId("degrade-notice")).toHaveCount(0);

  await typeMessage(page, "hello there");
  await page.getByTestId("send").click();

  // The user's message + an assistant turn that reaches a committed result.
  await expect(page.getByTestId("bubble-user")).toContainText("hello there");
  await expect(page.getByTestId("bubble-assistant")).toHaveAttribute("data-status", "done", {
    timeout: 30_000,
  });
});

test("a chat turn fails gracefully when the gateway is unreachable", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-chat").click();
  await page.getByTestId("chat-settings").locator("summary").click();
  await page.getByTestId("echo-preset").click();

  // Drop the gateway, then send: the turn must fail visibly, app stays usable.
  gw.stop();
  gw = undefined;

  await typeMessage(page, "are you there?");
  await page.getByTestId("send").click();

  await expect(page.getByTestId("bubble-assistant")).toHaveAttribute("data-status", "failed", {
    timeout: 30_000,
  });
  // The composer is still interactive (the app did not crash): typing re-arms send.
  await typeMessage(page, "still alive");
  await expect(page.getByTestId("send")).toBeEnabled();
});
