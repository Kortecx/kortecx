import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("Datasets: lists a seeded corpus + degrades cleanly without an embedder", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // Seed a corpus via the SDK's FFI-FREE client-vector path (no model / no Metal) —
  // the exact seam an external embedder (HuggingFace transformers) would use. The
  // gateway is built `--features hnsw`, so the dataset RPCs are wired.
  const kx = new KxClient(gw.endpoint);
  await kx.ingestDocuments("demo-corpus", [
    { content: new TextEncoder().encode("alpha"), embedding: [1, 0, 0, 0.1] },
    { content: new TextEncoder().encode("bravo"), embedding: [0, 1, 0, 0.1] },
    { content: new TextEncoder().encode("charlie"), embedding: [0, 0, 1, 0.1] },
  ]);
  kx.close();

  await connectConsole(page, gw);
  await page.getByTestId("nav-datasets").click();
  await expect(page.getByTestId("datasets-section")).toBeVisible();
  await expect(page.getByTestId("datasets-panel")).toBeVisible();

  // The seeded corpus renders as a CHIP (a button, NOT a controlled <select> — the
  // Playwright selectOption gotcha), auto-selected, with its doc count.
  const chip = page.getByTestId("dataset-pick-demo-corpus");
  await expect(chip).toBeVisible({ timeout: 30_000 });
  await expect(chip).toContainText("3 docs");
  await expect(chip).toHaveAttribute("aria-pressed", "true");

  // GR15 guard (PR-C2): the reference app's DatasetCard ships a Delete button, but
  // OUR gateway has NO DeleteDataset RPC — so the re-skin must expose NO delete
  // affordance (a faked, non-functional control would violate don't-fake-gaps).
  const section = page.getByTestId("datasets-section");
  await expect(section.getByRole("button", { name: /delete|remove|drop/i })).toHaveCount(0);
  await expect(section.getByTestId("dataset-delete")).toHaveCount(0);

  // Querying TEXT needs a server embedder this FFI-free gateway lacks → the
  // actionable no-embedder notice (FAILED_PRECONDITION), never a crash. Controlled
  // inputs are driven with click + pressSequentially (a bulk fill() can leave React
  // state stale — the recorded e2e gotcha).
  const queryInput = page.getByTestId("dataset-query-input");
  await queryInput.click();
  await queryInput.pressSequentially("alpha");
  await page.getByTestId("dataset-query-submit").click();
  await expect(page.getByText(/no embedding model on this gateway/i).first()).toBeVisible({
    timeout: 15_000,
  });

  // A text ingest from the UI surfaces the same guidance (the SDK client-vector path
  // is the FFI-free alternative).
  const nameInput = page.getByTestId("dataset-ingest-name");
  await nameInput.click();
  await nameInput.pressSequentially("from-ui");
  const textInput = page.getByTestId("dataset-ingest-text");
  await textInput.click();
  await textInput.pressSequentially("a document");
  await page.getByTestId("dataset-ingest-submit").click();
  await expect(page.getByText(/no embedding model on this gateway/i).first()).toBeVisible({
    timeout: 15_000,
  });
});
