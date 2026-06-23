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

test("Context-edit (POC-2): view an item body, edit it (ref changes), rename it, and the last item can't be removed", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // Author a one-item bundle from an uploaded text file.
  await page.getByTestId("nav-context").click();
  await expect(page.getByTestId("context-bundles")).toContainText("No context bundles yet", {
    timeout: 30_000,
  });
  const handle = page.getByTestId("context-bundle-handle");
  await handle.click();
  await handle.pressSequentially(HANDLE);
  await page.getByTestId("context-bundle-file-input").setInputFiles({
    name: "secret.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("the secret is ALPHA\n", "utf-8"),
  });
  await expect(page.getByTestId("context-bundle-staged")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("context-bundle-submit").click();
  await expect(page.getByTestId(`context-bundle-${HANDLE}`)).toBeVisible({ timeout: 30_000 });

  const key = `${HANDLE}-0`;
  const itemChip = page.getByTestId(`context-item-${key}`).getByTestId("digest-chip");
  const ref1 = await itemChip.getAttribute("title");

  // The last (only) item cannot be removed — that would empty the bundle.
  await expect(page.getByTestId(`context-item-remove-${key}`)).toBeDisabled();

  // Expand → the body resolves through the shared viewer (full bytes, uploads scope).
  await page.getByTestId(`context-item-toggle-${key}`).click();
  await expect(page.getByTestId(`context-item-body-${key}`)).toBeVisible({ timeout: 30_000 });

  // Edit the body in Monaco (drive by keyboard — fill() can't drive Monaco).
  await page.getByTestId(`context-item-edit-${key}`).click();
  const editor = page.getByTestId(`context-item-editor-${key}`);
  await expect(editor.locator(".monaco-editor")).toBeVisible({ timeout: 30_000 });
  await editor.locator(".monaco-editor").click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type("the secret is now OMEGA");
  await page.getByTestId(`context-item-save-${key}`).click();

  // Immutable CAS ⇒ the edit produced a NEW content ref: the item's chip changes.
  await expect.poll(async () => itemChip.getAttribute("title"), { timeout: 30_000 }).not.toBe(ref1);

  // Rename the item (a plain input → guarded re-upsert) and see the new label.
  await page.getByTestId(`context-item-name-${key}`).click();
  const nameInput = page.getByTestId(`context-item-name-input-${key}`);
  await nameInput.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type("renamed.txt");
  await page.getByTestId(`context-item-rename-save-${key}`).click();
  await expect(page.getByTestId(`context-item-name-${key}`)).toHaveText("renamed.txt", {
    timeout: 30_000,
  });
});
