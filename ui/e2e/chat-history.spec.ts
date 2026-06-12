import { expect, test } from "@playwright/test";
import { typeMessage } from "./fixtures/chat";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("a chat autosaves, survives a reload, and restores from History", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-chat").click();
  await page.getByTestId("chat-settings").locator("summary").click();
  await page.getByTestId("echo-preset").click();

  await typeMessage(page, "remember this chat");
  await page.getByTestId("send").click();
  await expect(page.getByTestId("bubble-assistant")).toHaveAttribute("data-status", "done", {
    timeout: 30_000,
  });

  // Reload: the live thread is gone (presentation state), the history is not.
  await page.reload();
  await connectConsole(page, gw);
  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("bubble-user")).toHaveCount(0);

  await page.getByTestId("chat-history-toggle").click();
  await expect(page.getByTestId("chat-history")).toBeVisible();
  const item = page.getByTestId("chat-history-item");
  await expect(item).toHaveCount(1);
  await expect(item).toContainText("remember this chat");

  // Restore: the saved messages come back into the live thread.
  await page.getByTestId("chat-history-load").click();
  await expect(page.getByTestId("chat-history")).toHaveCount(0);
  await expect(page.getByTestId("bubble-user")).toContainText("remember this chat");
  await expect(page.getByTestId("bubble-assistant")).toHaveAttribute("data-status", "done");

  // Forget it: the list empties.
  await page.getByTestId("chat-history-toggle").click();
  await page.getByTestId("chat-history-delete").click();
  await expect(page.getByTestId("chat-history-item")).toHaveCount(0);
});
