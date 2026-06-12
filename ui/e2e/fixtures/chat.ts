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
