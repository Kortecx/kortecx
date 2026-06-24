/**
 * POC-5d: the inline App "open" view — a single-App project IDE. A two-pane
 * layout (left: the {@link FileTree} over the App's CoW branch manifest; right:
 * the selected file via the read-only {@link CodeViewer}) plus a per-file agentic
 * Edit affordance (the same `editBranch` react-edit loop the Branches view uses)
 * and a toggle to the embedded {@link AppChat}. By convention the App's project
 * branch handle IS the App handle (one-App-one-branch).
 *
 * GR15 / D142 honesty: a LOCKED App (POC-5b) disables the Edit affordance with a
 * clear refusal notice — the runtime refuses agentic in-CAS edits at the
 * advance() chokepoint, and the UI never offers a control that can't fire. Every
 * state (loading / empty-project / not-found / missing-file / locked / edit
 * pending+error) is designed.
 */

import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useMemo, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useAppBranch, useAppFileContent } from "../../kx/use-app-files";
import { useApps, useRunApp } from "../../kx/use-apps";
import { useEditBranch } from "../../kx/use-branches";
import { buildFileTree } from "../../lib/file-tree";
import { inferLanguageFromPath } from "../../lib/monaco/infer-language";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { FileTree } from "../apps/FileTree";
import { AppChat } from "../chat/AppChat";
import { CodeViewer } from "../editor/CodeViewer";

type Pane = "files" | "chat";

export function AppDetailSection({ handle }: { handle: string }) {
  const navigate = useNavigate();
  const { apps } = useApps();
  const summary = apps.find((a) => a.handle === handle);
  const locked = summary?.locked ?? false;

  const branch = useAppBranch(handle);
  const runApp = useRunApp();
  const [pane, setPane] = useState<Pane>("files");
  const [selected, setSelected] = useState<{ path: string; contentRef: string } | null>(null);

  const items = branch.data?.items ?? [];
  const tree = useMemo(
    () => buildFileTree(items.map((it) => ({ path: it.path, contentRef: it.contentRef }))),
    [items],
  );

  function run(): void {
    runApp.mutate(
      { handle },
      {
        onSuccess: ({ instanceId }) => {
          void navigate({ to: "/workflows/$instanceId", params: { instanceId } });
        },
      },
    );
  }

  const runError = runApp.error ? toUiError(runApp.error) : null;

  return (
    <section className="screen app-detail" data-testid="app-detail">
      <div className="section-head">
        <div>
          <h1>{summary?.name ?? "App"}</h1>
          <code className="mono app-detail__handle" title={handle}>
            {handle}
          </code>
        </div>
        <div className="section-head__actions">
          {locked ? (
            <span
              className="chip chip--tag"
              data-testid="app-detail-locked"
              title="Agentic edits are refused"
            >
              🔒 Locked
            </span>
          ) : null}
          <button
            type="button"
            className="btn-primary"
            data-testid="app-detail-run"
            disabled={runApp.isPending}
            onClick={run}
          >
            {runApp.isPending ? "Running…" : "Run"}
          </button>
        </div>
      </div>

      {runError ? <ErrorNotice error={runError} onRetry={() => runApp.reset()} /> : null}

      <fieldset className="view-toggle" aria-label="App view" data-testid="app-detail-tabs">
        <button
          type="button"
          aria-pressed={pane === "files"}
          data-testid="app-tab-files"
          onClick={() => setPane("files")}
        >
          Files
        </button>
        <button
          type="button"
          aria-pressed={pane === "chat"}
          data-testid="app-tab-chat"
          onClick={() => setPane("chat")}
        >
          Chat
        </button>
      </fieldset>

      {pane === "chat" ? (
        <AppChat recipeHandle={handle} />
      ) : branch.isLoading ? (
        <EmptyState title="Loading project…" />
      ) : branch.isError ? (
        <ErrorNotice error={toUiError(branch.error)} onRetry={() => void branch.refetch()} />
      ) : branch.data === null ? (
        <EmptyState
          title="No project yet"
          detail="This App has no scaffolded project branch. Use New App to scaffold one, or run kx app scaffold."
        />
      ) : (
        <div className="app-detail__panes" data-testid="app-detail-panes">
          <aside className="app-detail__tree">
            <FileTree
              nodes={tree}
              selectedPath={selected?.path ?? null}
              onSelect={(path, contentRef) => setSelected({ path, contentRef })}
            />
          </aside>
          <div className="app-detail__file">
            {selected ? (
              <FilePane
                handle={handle}
                path={selected.path}
                contentRef={selected.contentRef}
                locked={locked}
                onEdited={() => void branch.refetch()}
              />
            ) : (
              <EmptyState
                title="Select a file"
                detail="Pick a file in the tree to view its content."
              />
            )}
          </div>
        </div>
      )}
    </section>
  );
}

/** The right pane: the selected file's body + a per-file agentic Edit (gated by
 *  the App lock). */
function FilePane({
  handle,
  path,
  contentRef,
  locked,
  onEdited,
}: {
  handle: string;
  path: string;
  contentRef: string;
  locked: boolean;
  onEdited: () => void;
}) {
  const body = useAppFileContent(handle, path, contentRef, true);
  const edit = useEditBranch();
  const [editing, setEditing] = useState(false);
  const [instruction, setInstruction] = useState("");
  const editError = edit.error ? toUiError(edit.error) : null;

  const submit = (): void => {
    const trimmed = instruction.trim();
    if (trimmed.length === 0) {
      return;
    }
    edit.mutate(
      { handle, path, instruction: trimmed },
      {
        onSuccess: () => {
          setEditing(false);
          setInstruction("");
          onEdited();
        },
      },
    );
  };

  return (
    <m.div
      className="app-file"
      data-testid={`app-file-${path}`}
      variants={fadeUp}
      initial="hidden"
      animate="show"
    >
      <div className="app-file__head">
        <code className="mono app-file__path">{path}</code>
        {locked ? (
          <span className="muted app-file__locked" data-testid="app-locked-notice" role="note">
            This App is locked — agentic edits are refused. Unlock it in Policies to edit.
          </span>
        ) : (
          <button
            type="button"
            className="btn-ghost"
            data-testid="app-file-edit"
            disabled={edit.isPending}
            onClick={() => {
              setEditing((e) => !e);
              setInstruction("");
            }}
          >
            {editing ? "Cancel" : "Edit"}
          </button>
        )}
      </div>

      {body.isLoading ? (
        <EmptyState title="Loading file…" />
      ) : body.isError ? (
        <ErrorNotice error={toUiError(body.error)} onRetry={() => void body.refetch()} />
      ) : body.data?.missing ? (
        <EmptyState
          title="File not found"
          detail="This path is not in the App's branch (or not owned)."
        />
      ) : (
        <CodeViewer
          value={body.data?.text ?? ""}
          language={inferLanguageFromPath(path)}
          testId={`app-file-body-${path}`}
          ariaLabel={`File ${path}`}
          height={Math.min(560, Math.max(180, (body.data?.text.split("\n").length ?? 1) * 19 + 24))}
        />
      )}

      {!locked && editing ? (
        <div className="app-file__editor" data-testid="app-file-edit-form">
          <textarea
            className="input"
            data-testid="app-file-edit-instruction"
            placeholder="Describe the change — the agent rewrites this file in-CAS (the host is never written)…"
            rows={2}
            value={instruction}
            disabled={edit.isPending}
            onChange={(e) => setInstruction(e.target.value)}
          />
          <button
            type="button"
            className="btn-primary"
            data-testid="app-file-edit-submit"
            disabled={edit.isPending || instruction.trim().length === 0}
            onClick={submit}
          >
            {edit.isPending ? "Editing…" : "Run edit"}
          </button>
        </div>
      ) : null}

      {editError ? (
        <p className="field-error" data-testid="app-file-edit-error" role="alert">
          {editError.message}
        </p>
      ) : null}
    </m.div>
  );
}
