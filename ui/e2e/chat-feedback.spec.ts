import { expect, test } from "@playwright/test";
import { seedEchoBacking, typeMessage } from "./fixtures/chat";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

// The echo backing is seeded via localStorage before connect (see each test) — no
// in-panel preset button any more; this just drives a settled echo turn.
async function sendEcho(page: import("@playwright/test").Page, text: string) {
  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();
  await typeMessage(page, text);
  await page.getByTestId("send").click();
  await expect(page.getByTestId("bubble-assistant")).toHaveAttribute("data-status", "done", {
    timeout: 30_000,
  });
}

test("a settled answer carries copy + 👍/👎; rating SENDS SubmitFeedback to the gateway", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await seedEchoBacking(page);
  await connectConsole(page, gw);
  await sendEcho(page, "rate me");

  // The action row is present on a settled answer.
  await expect(page.getByTestId("msg-actions")).toBeVisible();
  await expect(page.getByTestId("msg-copy")).toBeVisible();
  const up = page.getByTestId("msg-feedback-up");
  const down = page.getByTestId("msg-feedback-down");
  await expect(up).toBeVisible();

  // Clicking 👍 fires a SubmitFeedback RPC (the UI really sends it) and selects.
  const [req] = await Promise.all([
    page.waitForRequest((r) => r.url().includes("/SubmitFeedback")),
    up.click(),
  ]);
  expect(req.method()).toBe("POST");
  await expect(up).toHaveAttribute("aria-pressed", "true");

  // Re-rating 👎 overwrites the optimistic selection.
  await Promise.all([
    page.waitForRequest((r) => r.url().includes("/SubmitFeedback")),
    down.click(),
  ]);
  await expect(down).toHaveAttribute("aria-pressed", "true");
  await expect(up).toHaveAttribute("aria-pressed", "false");

  // Copy does not throw (clipboard may be unavailable headless — degrade safe).
  await page.getByTestId("msg-copy").click();
});

test("New Chat is read-only + RAG-grounded — header Context attach, no Agent toggle, and an attach menu with no tools/context/dataset placeholder", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);
  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();

  // Wave-4: the header "Context" attach button is the read-only grounding affordance
  // (the standalone dataset grounding bar is gone), and there is NO Agent-task toggle —
  // the mutate-capable agentic chat lives in Apps.
  await expect(page.getByTestId("chat-grounding")).toHaveCount(0);
  await expect(page.getByTestId("chat-grounding-add")).toBeVisible();
  await expect(page.getByTestId("chat-mode")).toHaveCount(0);

  await page.getByTestId("attach-btn").click();
  await expect(page.getByTestId("attach-menu")).toBeVisible();
  await expect(page.getByTestId("attach-upload")).toBeEnabled();
  // Read-only: NO Tools category (the mutate path is gone), NO Context category
  // (context selection moved to the grounding bar), NO "Dataset" placeholder
  // (grounding is first-class). Blueprint stays honest-disabled (don't-fake-gaps).
  await expect(page.getByTestId("attach-tool-group")).toHaveCount(0);
  await expect(page.getByTestId("attach-context-group")).toHaveCount(0);
  await expect(page.getByTestId("attach-dataset")).toHaveCount(0);
  await expect(page.getByTestId("attach-blueprint")).toBeDisabled();

  // Escape closes the menu.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("attach-menu")).toHaveCount(0);

  // The grounding bar's "+ Context" popover lists context files (honest empty on a
  // fresh serve) — the first-class replacement for the old attach-menu category.
  await page.getByTestId("chat-grounding-add").click();
  await expect(page.getByTestId("chat-grounding-menu")).toBeVisible();
  await expect(page.getByTestId("chat-grounding-empty")).toBeVisible();
});

test("Export downloads the chat as JSON", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await seedEchoBacking(page);
  await connectConsole(page, gw);
  await sendEcho(page, "export me");

  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByTestId("chat-export").click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/^kortecx-chat-.*\.json$/);
});
