import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

// A 1×1 transparent PNG (the smallest valid attach fixture).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  "base64",
);

test("attach uploads a PNG (PutContent), previews it, dedups a re-pick, and sends display-only over echo", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();
  // The model-free echo recipe: its form has NO image_ref slot, so the
  // attachment must stay display-only and the send must still succeed.
  await page.getByTestId("chat-settings").locator("summary").click();
  await page.getByTestId("echo-preset").click();

  // Attach → the chip appears, the upload lands, the DigestChip shows the
  // SERVER-derived ref.
  await page.getByTestId("attach-input").setInputFiles({
    name: "pixel.png",
    mimeType: "image/png",
    buffer: PNG_1X1,
  });
  const chip = page.getByTestId("attachment-chip");
  await expect(chip).toHaveAttribute("data-status", "ready", { timeout: 15_000 });
  await expect(chip.getByTestId("digest-chip")).toBeVisible();
  await expect(chip.locator("img")).toBeVisible();

  // Re-pick the IDENTICAL file: content-addressed — one chip, dedup badge.
  await page.getByTestId("attach-input").setInputFiles({
    name: "pixel.png",
    mimeType: "image/png",
    buffer: PNG_1X1,
  });
  await expect(page.getByTestId("attachment-dedup")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("attachment-chip")).toHaveCount(1);

  // Send: the user bubble carries the attachment (preview + DigestChip) and the
  // echo turn still commits (display-only path — no undeclared arg was sent).
  await page.getByLabel(/^message$/i).fill("what is attached?");
  await page.getByTestId("send").click();
  await expect(page.getByTestId("bubble-attachments")).toBeVisible();
  await expect(page.getByTestId("bubble-user").getByTestId("digest-chip")).toBeVisible();
  await expect(page.getByTestId("bubble-assistant")).toHaveAttribute("data-status", "done", {
    timeout: 30_000,
  });
});

test("a failed turn offers retry with identical args and recovers when the gateway returns", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-chat").click();
  await page.getByTestId("chat-settings").locator("summary").click();
  await page.getByTestId("echo-preset").click();

  // Fail the first turn by pointing chat at a recipe the gateway cannot run.
  await page.getByLabel(/blueprint handle/i).fill("kx/recipes/does-not-exist");
  await page.getByLabel(/^message$/i).fill("retry me");
  await page.getByTestId("send").click();
  await expect(page.getByTestId("bubble-assistant")).toHaveAttribute("data-status", "failed", {
    timeout: 30_000,
  });

  // Fix the handle, retry the SAME turn (identical text re-dispatches).
  await page.getByTestId("echo-preset").click();
  await page.getByTestId("retry-turn").click();
  await expect(page.getByTestId("bubble-assistant")).toHaveAttribute("data-status", "done", {
    timeout: 30_000,
  });
  await expect(page.getByTestId("bubble-user")).toContainText("retry me");
});
