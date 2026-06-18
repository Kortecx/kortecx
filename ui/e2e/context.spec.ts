import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

const SPEC_FILE = Buffer.from("The launch codename is FALCON-NINE-ZULU.\n", "utf-8");
const HANDLE = "team/ctx/spec";

test("Context: author a bundle from an uploaded file, see it listed, attach it in chat, then delete", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // The section opens to an honest empty state on a fresh (per-spawn) gateway.
  await page.getByTestId("nav-context").click();
  await expect(page.getByTestId("context-section")).toBeVisible();
  await expect(page.getByTestId("context-bundles")).toContainText("No context bundles yet", {
    timeout: 30_000,
  });

  // Author a bundle: a handle + one uploaded file (PutContent → server-derived ref).
  // The handle input is React-controlled — click + pressSequentially, never fill().
  const handle = page.getByTestId("context-bundle-handle");
  await handle.click();
  await handle.pressSequentially(HANDLE);
  await page.getByTestId("context-bundle-file-input").setInputFiles({
    name: "spec.md",
    mimeType: "text/markdown",
    buffer: SPEC_FILE,
  });
  // The staged item appears once the upload lands (with its DigestChip).
  await expect(page.getByTestId("context-bundle-staged")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("context-bundle-submit").click();

  // The bundle now lists with its server-derived bundle ref + the item.
  const card = page.getByTestId(`context-bundle-${HANDLE}`);
  await expect(card).toBeVisible({ timeout: 30_000 });
  await expect(card.getByTestId("digest-chip").first()).toBeVisible();
  await expect(card).toContainText("spec.md");

  // The chat composer's LIVE Context attach category lists the bundle; attaching
  // it shows the pending-context chip (the wire from the section to the turn).
  await page.getByTestId("nav-chat").click();
  await expect(page.getByTestId("chat-panel")).toBeVisible();
  await page.getByTestId("attach-btn").click();
  await expect(page.getByTestId("attach-context-group")).toBeVisible();
  await page.getByTestId(`attach-context-option-${HANDLE}`).click();
  await expect(page.getByTestId(`chat-context-${HANDLE}`)).toBeVisible();
  // Detaching clears the chip.
  await page.getByTestId(`chat-context-remove-${HANDLE}`).click();
  await expect(page.getByTestId(`chat-context-${HANDLE}`)).toHaveCount(0);

  // Delete the bundle from the section → it leaves the list.
  await page.getByTestId("nav-context").click();
  await page.getByTestId(`context-bundle-delete-${HANDLE}`).click();
  await expect(page.getByTestId(`context-bundle-${HANDLE}`)).toHaveCount(0, { timeout: 30_000 });
});
