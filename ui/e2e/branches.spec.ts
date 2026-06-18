import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

const HANDLE = "team/workspace/main";
const FORK = "team/workspace/feature";

test("Branches (D155): snapshot a confined file, see the manifest, fork a sub-branch, then delete", async ({
  page,
}) => {
  // An operator read root with one file — the only thing snapshot reads.
  const fsRoot = await mkdtemp(path.join(tmpdir(), "kxbranch-"));
  await writeFile(path.join(fsRoot, "notes.md"), "# Project notes\nthe codename is FALCON.\n");

  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN, fsRoot });
  await connectConsole(page, gw);

  // The section opens to an honest empty state on a fresh (per-spawn) gateway.
  await page.getByTestId("nav-branches").click();
  await expect(page.getByTestId("branches-section")).toBeVisible();
  await expect(page.getByTestId("branches")).toContainText("No branches yet", { timeout: 30_000 });

  // Snapshot one confined path into a new branch (handle is React-controlled —
  // click + pressSequentially, never fill()).
  const handle = page.getByTestId("branch-handle");
  await handle.click();
  await handle.pressSequentially(HANDLE);
  const pathDraft = page.getByTestId("branch-path-draft");
  await pathDraft.click();
  await pathDraft.pressSequentially("notes.md");
  await page.getByTestId("branch-add-path").click();
  await expect(page.getByTestId("branch-staged-paths")).toContainText("notes.md");
  await page.getByTestId("branch-snapshot-submit").click();

  // The branch lists with its server-derived branch ref + the {path → ref} entry.
  const card = page.getByTestId(`branch-${HANDLE}`);
  await expect(card).toBeVisible({ timeout: 30_000 });
  await expect(card.getByTestId("digest-chip").first()).toBeVisible();
  await expect(card).toContainText("notes.md");
  await expect(card).toContainText("1 file");

  // Fork a point-in-time CoW sub-branch (create, no snapshot needed).
  const forkHandle = page.getByTestId("branch-handle");
  await forkHandle.click();
  await forkHandle.pressSequentially(FORK);
  const parent = page.getByTestId("branch-parent");
  await parent.click();
  await parent.pressSequentially(HANDLE);
  await page.getByTestId("branch-create-submit").click();

  // The fork lists, inherits the parent's file, and shows its lineage chip.
  const forkCard = page.getByTestId(`branch-${FORK}`);
  await expect(forkCard).toBeVisible({ timeout: 30_000 });
  await expect(forkCard).toContainText(HANDLE); // the "← parent" badge
  await expect(forkCard).toContainText("notes.md");

  // Delete the fork → it leaves the list (the parent stays).
  await page.getByTestId(`branch-delete-${FORK}`).click();
  await expect(page.getByTestId(`branch-${FORK}`)).toHaveCount(0, { timeout: 30_000 });
  await expect(page.getByTestId(`branch-${HANDLE}`)).toBeVisible();
});
