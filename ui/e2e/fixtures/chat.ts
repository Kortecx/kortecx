import type { Page } from "@playwright/test";

/**
 * Type into the chat composer. The composer is a Monaco surface in the real
 * browser (PR-1.1), so label-based `fill()` no longer applies — click the
 * editor (or its Suspense textarea fallback; same testid) and type through the
 * keyboard, which both surfaces accept.
 */
export async function typeMessage(page: Page, text: string): Promise<void> {
  const input = page.getByTestId("composer-input");
  await input.click();
  await page.keyboard.type(text);
}

/**
 * Seed the persisted chat backing BEFORE the app loads. Wave-4 removed the in-panel
 * `echo`-preset button + the Advanced recipe-handle reveal (New Chat is a chat, not a
 * recipe console), so a test points chat at a specific backing by seeding the same
 * `kortecx.ui.chat` localStorage schema that `loadChatSettings` reads on mount. Registers
 * an init script for every navigation in the test — CALL IT BEFORE `connectConsole`.
 */
export async function seedChatBacking(
  page: Page,
  backing: { handle: string; promptKey: string },
): Promise<void> {
  await page.addInitScript((b) => {
    localStorage.setItem(
      "kortecx.ui.chat",
      JSON.stringify({
        handle: b.handle,
        promptKey: b.promptKey,
        showThinking: true,
        showReasoning: true,
        autoscroll: true,
      }),
    );
  }, backing);
}

/** Seed the deterministic, model-free `echo` recipe backing (the old `echo-preset` click). */
export async function seedEchoBacking(page: Page): Promise<void> {
  await seedChatBacking(page, { handle: "kx/recipes/echo", promptKey: "topic" });
}
