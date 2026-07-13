import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

/**
 * PR-A: New Chat is READ-ONLY, RAG-grounded. This spec asserts the read-only SHAPE +
 * the first-class grounding affordance in BOTH themes over a real `--features hnsw`
 * gateway with a seeded corpus. A LIVE grounded answer + rendered citations need a
 * served model + embedder (the manual live-serve recipe) — this FFI-free gateway has
 * neither, so we assert the grounding UI, not the answer.
 */

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("New Chat grounds over a dataset (read-only shape, first-class picker) in BOTH themes", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // Seed a corpus via the SDK's FFI-FREE client-vector path (no model / no Metal).
  const kx = new KxClient(gw.endpoint);
  await kx.ingestDocuments("demo-corpus", [
    { content: new TextEncoder().encode("alpha"), embedding: [1, 0, 0, 0.1] },
    { content: new TextEncoder().encode("bravo"), embedding: [0, 1, 0, 0.1] },
    { content: new TextEncoder().encode("charlie"), embedding: [0, 0, 1, 0.1] },
  ]);
  kx.close();

  await connectConsole(page, gw);
  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();

  // Read-only shape: the grounding bar is the headline; there is NO Agent toggle.
  await expect(page.getByTestId("chat-grounding")).toBeVisible();
  await expect(page.getByTestId("chat-mode")).toHaveCount(0);

  // The seeded corpus is selectable in the grounding bar's dataset picker (a real
  // <select> — selectOption is safe here, unlike the section's chip buttons).
  const select = page.getByTestId("dataset-picker-select");
  await expect(page.getByTestId("dataset-picker")).toBeVisible({ timeout: 30_000 });
  await select.selectOption("demo-corpus");

  // The summary + the recipe note both reflect the grounding (chat-rag over the corpus).
  await expect(page.getByTestId("chat-grounded-on")).toHaveText("demo-corpus");
  await expect(page.getByTestId("chat-grounding-summary")).toContainText("demo-corpus");

  // Both themes (D142 / GR13): the grounding bar + the selected-dataset summary stay
  // visible and legible in light AND dark.
  for (const theme of ["light", "dark"] as const) {
    const current = await page.locator("html").getAttribute("data-theme");
    if (current !== theme) {
      await page.getByTestId("theme-toggle").click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.getByTestId("chat-grounding")).toBeVisible();
    await expect(page.getByTestId("chat-grounding-summary")).toContainText("demo-corpus");
  }
});
