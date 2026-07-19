/**
 * POC-5a / POC-6: the honest, polled agentic-creation view. After "New App"
 * launches a server-side agentic scaffold, this polls `GetScaffoldStatus` and
 * renders an IDE-shaped surface: a live FILE TREE (done ✓ / writing ◐ / pending ·)
 * built from the server's DYNAMIC manifest, and a Monaco pane that STREAMS the
 * file currently being authored token-by-token (POC-6 — `useTokenStream` over the
 * write mote the server surfaces), swapping to the committed body once a file
 * lands. Driven ENTIRELY by the server's real `filesDone` / `filesPending` +
 * phase + live-writing ids (GR15: never a timer, never fabricated progress). On
 * `failed` it shows the server `detail` and keeps the partial files; on `done` it
 * offers an "Open" CTA into the full App IDE. Polling stops the instant the phase
 * is terminal.
 */

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { queryKeys } from "../../kx/query-keys";
import { useAppBranch, useAppFileContent } from "../../kx/use-app-files";
import { useInvalidateOnScaffoldDone, useScaffoldStatus } from "../../kx/use-scaffold-app";
import { useTokenStream } from "../../kx/use-token-stream";
import { buildFileTree } from "../../lib/file-tree";
import { inferLanguageFromPath } from "../../lib/monaco/infer-language";
import { type DerivedScaffold, deriveScaffoldStatus } from "../../lib/scaffold-status";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { FileTree } from "../apps/FileTree";
import { CodeViewer } from "../editor/CodeViewer";

const EDITOR_HEIGHT = 460;

export function ScaffoldProgress({
  branchHandle,
  appHandle,
}: {
  /** The App's project branch handle to poll (== the App handle by convention). */
  branchHandle: string;
  /** The App handle to navigate to on completion. */
  appHandle: string;
}) {
  const navigate = useNavigate();
  const invalidate = useInvalidateOnScaffoldDone();
  const { endpoint } = useConnection();
  const qc = useQueryClient();
  // Poll while a phase is non-terminal; the query halts its own interval on
  // done/failed, and we flip `enabled` off so it never re-arms.
  const status = useScaffoldStatus(branchHandle, true);
  const data = status.data;
  const derived: DerivedScaffold | null = data ? deriveScaffoldStatus(data) : null;

  // The branch manifest (for a done file's content ref, so a selected done file
  // renders its committed body). Kept fresh: whenever a new file starts writing,
  // the previous one just committed — refetch the branch so its ref appears.
  const branch = useAppBranch(appHandle);
  const writingPath = derived?.writingPath;
  const prevWriting = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (writingPath !== prevWriting.current) {
      prevWriting.current = writingPath;
      void qc.invalidateQueries({ queryKey: queryKeys.appBranch(endpoint, appHandle) });
    }
  }, [writingPath, qc, endpoint, appHandle]);

  // Selection: `null` = auto-follow the writing file; a path = pinned by the user.
  const [pinned, setPinned] = useState<string | null>(null);

  // On completion, refresh the catalog + branch caches so the new tree appears.
  useEffect(() => {
    if (derived?.complete) {
      invalidate(branchHandle);
    }
  }, [derived?.complete, branchHandle, invalidate]);

  if (status.isLoading) {
    return <EmptyState title="Starting the scaffold…" />;
  }
  if (status.isError) {
    return <ErrorNotice error={toUiError(status.error)} onRetry={() => void status.refetch()} />;
  }
  if (!derived || !data) {
    return null;
  }

  const heading =
    derived.phase === "planning"
      ? "Planning the project…"
      : derived.phase === "writing"
        ? "Writing the project files…"
        : derived.failed
          ? "Scaffold failed"
          : derived.complete
            ? "Project ready"
            : "Scaffolding…";

  const refOf = (path: string): string | null =>
    branch.data?.items.find((i) => i.path === path)?.contentRef ?? null;

  // The live file tree from the server's dynamic rows (done/writing/pending).
  const nodes = buildFileTree(
    derived.rows.map((r) => ({
      path: r.path,
      state: r.state,
      contentRef: r.state === "done" ? (refOf(r.path) ?? undefined) : undefined,
    })),
  );

  // Auto-follow the writing file until the user pins one; then hold the pin.
  const firstDone = derived.rows.find((r) => r.state === "done")?.path;
  const selectedPath = pinned ?? derived.writingPath ?? firstDone ?? null;

  return (
    <div
      className="scaffold-progress scaffold-ide"
      data-testid="scaffold-progress"
      data-phase={derived.phase}
    >
      <div className="scaffold-progress__head">
        <h3 className="scaffold-progress__title">{heading}</h3>
        <code className="mono scaffold-progress__handle" title={appHandle}>
          {appHandle}
        </code>
      </div>

      {derived.rows.length === 0 ? (
        <EmptyState title="The model is planning the project's files…" />
      ) : (
        <div className="scaffold-ide__body">
          <div className="scaffold-ide__tree" data-testid="scaffold-tree">
            <FileTree
              nodes={nodes}
              selectedPath={selectedPath}
              onSelect={(path) => setPinned(path)}
            />
          </div>
          <div className="scaffold-ide__editor">
            <ScaffoldEditorPane
              selectedPath={selectedPath}
              derived={derived}
              appHandle={appHandle}
              contentRef={selectedPath ? refOf(selectedPath) : null}
            />
          </div>
        </div>
      )}

      {derived.failed ? (
        <p className="field-error" data-testid="scaffold-failed" role="alert">
          The scaffold did not finish:{" "}
          {data.detail || "the agent stopped without writing every file."} The files written so far
          are kept above — re-run "New App" to resume.
        </p>
      ) : null}

      {derived.complete ? (
        <div className="scaffold-progress__done">
          <p className="muted" data-testid="scaffold-done">
            The project is written. Open the App to browse and edit its full tree.
          </p>
          <button
            type="button"
            className="btn-primary"
            data-testid="scaffold-open"
            onClick={() => void navigate({ to: "/apps/$handle", params: { handle: appHandle } })}
          >
            Open App
          </button>
        </div>
      ) : null}
    </div>
  );
}

/** The right pane: stream the writing file, show a committed body, or a placeholder. */
function ScaffoldEditorPane({
  selectedPath,
  derived,
  appHandle,
  contentRef,
}: {
  selectedPath: string | null;
  derived: DerivedScaffold;
  appHandle: string;
  contentRef: string | null;
}) {
  const isWriting =
    selectedPath !== null &&
    selectedPath === derived.writingPath &&
    derived.writingInstanceId !== undefined &&
    derived.writingMoteId !== undefined;

  if (isWriting) {
    return (
      <StreamingFilePane
        path={selectedPath as string}
        instanceId={derived.writingInstanceId as string}
        moteId={derived.writingMoteId as string}
      />
    );
  }
  if (selectedPath === null) {
    return <EmptyState title="The project files will stream in here as they're written." />;
  }
  const state = derived.rows.find((r) => r.path === selectedPath)?.state;
  if (state === "done") {
    return <DoneFilePane handle={appHandle} path={selectedPath} contentRef={contentRef} />;
  }
  return <EmptyState title={`${selectedPath} — waiting to be written`} />;
}

/** POC-6: stream ONE file's decode into a read-only Monaco as the model authors it. */
function StreamingFilePane({
  path,
  instanceId,
  moteId,
}: {
  path: string;
  instanceId: string;
  moteId: string;
}) {
  const { text, streaming } = useTokenStream(instanceId, moteId, true);
  return (
    <div className="scaffold-ide__pane" data-testid="scaffold-stream" data-writing-path={path}>
      <div className="scaffold-ide__pane-head">
        <code className="mono">{path}</code>
        <span className="muted scaffold-ide__streaming">
          {streaming ? "streaming…" : "authoring…"}
        </span>
      </div>
      <CodeViewer
        value={text}
        language={inferLanguageFromPath(path)}
        height={EDITOR_HEIGHT}
        testId="scaffold-stream-code"
        ariaLabel={`Authoring ${path}`}
      />
    </div>
  );
}

/** Show a committed project file's body (a done file the user selected). */
function DoneFilePane({
  handle,
  path,
  contentRef,
}: {
  handle: string;
  path: string;
  contentRef: string | null;
}) {
  const body = useAppFileContent(handle, path, contentRef, contentRef !== null);
  return (
    <div className="scaffold-ide__pane" data-testid="scaffold-file" data-file-path={path}>
      <div className="scaffold-ide__pane-head">
        <code className="mono">{path}</code>
        <span className="muted">written</span>
      </div>
      <CodeViewer
        value={body.data?.text ?? ""}
        language={inferLanguageFromPath(path)}
        height={EDITOR_HEIGHT}
        testId="scaffold-file-code"
        ariaLabel={`File ${path}`}
      />
    </div>
  );
}
