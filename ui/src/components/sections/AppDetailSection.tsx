/**
 * POC-5d: the single-App IDE — a full-screen workspace (the `.screen` shell, like
 * the run detail) with three URL-addressable tabs:
 *  - **Files**: the {@link FileTree} over the App's CoW branch manifest + a file pane
 *    that VIEWS a file (read-only Monaco), edits it DIRECTLY (typed Monaco →
 *    PutContent → AdvanceBranch), or edits it AGENTICALLY with a REVIEW/DIFF GATE
 *    (propose → diff → approve/reject; closes T-AGENTIC-EDIT-REVIEW-GATE);
 *  - **Lineage**: the editable blueprint graph ({@link AppLineageSection});
 *  - **Chat**: the embedded App-scoped {@link AppChat}.
 * The header carries the App name, a Lock chip, and Run (opens {@link AppRunDrawer}).
 *
 * GR15 / D142 honesty: a LOCKED App disables every WRITE affordance (direct save +
 * agentic edit + structure save) with a clear notice — the runtime refuses the write
 * at the AdvanceBranch / SaveApp chokepoints (LOCKED_BRANCH), so the UI never offers
 * a control that can't fire. Every state (loading / empty-project / not-found /
 * missing-file / locked / pending / error) is designed.
 */

import { m } from "framer-motion";
import { useEffect, useMemo, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useAppBranch, useAppFileContent, useSaveFile } from "../../kx/use-app-files";
import { useApp } from "../../kx/use-apps";
import { useAdvanceBranch, useEditBranchPropose } from "../../kx/use-branches";
import { buildFileTree } from "../../lib/file-tree";
import { inferLanguageFromPath } from "../../lib/monaco/infer-language";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AppRunDrawer } from "../apps/AppRunDrawer";
import { FileTree } from "../apps/FileTree";
import { SkillsRail } from "../apps/SkillsRail";
import { AppChat } from "../chat/AppChat";
import { CodeViewer } from "../editor/CodeViewer";
import { DiffViewer } from "../editor/DiffViewer";
import { MonacoMount } from "../editor/MonacoMount";
import { AppLineageSection } from "./AppLineageSection";

const TABS = ["files", "lineage", "chat"] as const;
export type IdeTab = (typeof TABS)[number];

export function AppDetailSection({
  handle,
  tab: tabProp,
  path: pathProp,
  onTab,
  onPath,
}: {
  handle: string;
  /** Controlled active tab (the route binds `?tab=`); uncontrolled when absent. */
  tab?: IdeTab;
  /** Controlled selected file path (the route binds `?path=`). */
  path?: string;
  onTab?: (tab: IdeTab) => void;
  onPath?: (path: string | undefined) => void;
}) {
  const app = useApp(handle);
  const summary = app.data?.summary;
  const locked = summary?.locked ?? false;

  const [tabState, setTabState] = useState<IdeTab>("files");
  const [pathState, setPathState] = useState<string | undefined>(undefined);
  const tab = tabProp ?? tabState;
  const selectedPath = pathProp ?? pathState;
  const setTab = (t: IdeTab) => (onTab ? onTab(t) : setTabState(t));
  const setPath = (p: string | undefined) => (onPath ? onPath(p) : setPathState(p));

  const [runOpen, setRunOpen] = useState(false);

  const branch = useAppBranch(handle);
  const items = branch.data?.items ?? [];
  const tree = useMemo(
    () => buildFileTree(items.map((it) => ({ path: it.path, contentRef: it.contentRef }))),
    [items],
  );
  const selected = selectedPath ? (items.find((it) => it.path === selectedPath) ?? null) : null;

  return (
    <section className="screen app-detail" data-testid="app-detail">
      <div className="screen__head">
        <div>
          <h1>{summary?.name ?? "App"}</h1>
          <code className="mono app-detail__handle" title={handle}>
            {handle}
          </code>
        </div>
        <div className="screen__head-actions">
          {locked ? (
            <span
              className="chip chip--tag"
              data-testid="app-detail-locked"
              title="Edits are refused"
            >
              🔒 Locked
            </span>
          ) : null}
          <button
            type="button"
            className="btn-primary"
            data-testid="app-detail-run"
            onClick={() => setRunOpen(true)}
          >
            Run
          </button>
        </div>
      </div>

      {app.data ? (
        <SkillsRail handle={handle} envelope={app.data.envelope} locked={locked} />
      ) : null}

      <fieldset className="view-toggle" aria-label="App view" data-testid="app-detail-tabs">
        {TABS.map((t) => (
          <button
            key={t}
            type="button"
            aria-pressed={tab === t}
            data-testid={`app-tab-${t}`}
            onClick={() => setTab(t)}
          >
            {t === "files" ? "Files" : t === "lineage" ? "Lineage" : "Chat"}
          </button>
        ))}
      </fieldset>

      {tab === "chat" ? (
        <AppChat recipeHandle={handle} />
      ) : tab === "lineage" ? (
        <AppLineageSection handle={handle} locked={locked} />
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
              onSelect={(path) => setPath(path)}
            />
          </aside>
          <div className="app-detail__file">
            {selected ? (
              <FilePane
                handle={handle}
                path={selected.path}
                contentRef={selected.contentRef}
                locked={locked}
                onCommitted={() => void branch.refetch()}
              />
            ) : (
              <EmptyState
                title="Select a file"
                detail="Pick a file in the tree to view or edit it."
              />
            )}
          </div>
        </div>
      )}

      {runOpen ? <AppRunDrawer handle={handle} onClose={() => setRunOpen(false)} /> : null}
    </section>
  );
}

type Mode = "view" | "direct" | "agentic";

/** The right pane: view a file, edit it directly (Monaco → save), or edit it
 *  agentically through the review/diff gate. Lock-gated (GR15). */
function FilePane({
  handle,
  path,
  contentRef,
  locked,
  onCommitted,
}: {
  handle: string;
  path: string;
  contentRef: string;
  locked: boolean;
  onCommitted: () => void;
}) {
  const body = useAppFileContent(handle, path, contentRef, true);
  const saveFile = useSaveFile();
  const propose = useEditBranchPropose();
  const advance = useAdvanceBranch();

  const [mode, setMode] = useState<Mode>("view");
  const [draft, setDraft] = useState("");
  const [instruction, setInstruction] = useState("");
  const language = inferLanguageFromPath(path);
  const text = body.data?.text ?? "";

  // Re-base on a new file selection / a committed change: drop to view + clear drafts.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-base when the file or its ref changes
  useEffect(() => {
    setMode("view");
    setDraft("");
    setInstruction("");
    propose.reset();
  }, [path, contentRef]);

  const dirty = mode === "direct" && draft !== text;
  const proposal = propose.data ?? null;

  function startDirect(): void {
    setDraft(text);
    setMode("direct");
  }
  function saveDirect(): void {
    saveFile.mutate(
      { handle, path, text: draft },
      {
        onSuccess: () => {
          setMode("view");
          onCommitted();
        },
      },
    );
  }
  function runPropose(): void {
    const trimmed = instruction.trim();
    if (trimmed.length === 0) {
      return;
    }
    propose.mutate({ handle, path, instruction: trimmed });
  }
  function approve(): void {
    if (!proposal) {
      return;
    }
    advance.mutate(
      { handle, path, contentRef: proposal.resultRef },
      {
        onSuccess: () => {
          propose.reset();
          setInstruction("");
          setMode("view");
          onCommitted();
        },
      },
    );
  }
  function reject(): void {
    propose.reset();
  }

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
            This App is locked — edits are refused. Unlock it in Security › Policies to edit.
          </span>
        ) : mode === "view" ? (
          <div className="app-file__actions">
            <button
              type="button"
              className="btn-ghost"
              data-testid="app-file-edit-direct"
              onClick={startDirect}
            >
              Edit
            </button>
            <button
              type="button"
              className="btn-ghost"
              data-testid="app-file-edit-agentic"
              onClick={() => setMode("agentic")}
            >
              Ask agent
            </button>
          </div>
        ) : (
          <button
            type="button"
            className="btn-ghost"
            data-testid="app-file-cancel"
            onClick={() => {
              setMode("view");
              propose.reset();
            }}
          >
            Cancel
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
      ) : mode === "direct" ? (
        <div className="app-file__editor" data-testid="app-file-direct-editor">
          <MonacoMount
            value={draft}
            language={language}
            readOnly={false}
            onChange={setDraft}
            testId={`app-file-monaco-${path}`}
            ariaLabel={`Edit ${path}`}
            height={Math.min(560, Math.max(220, text.split("\n").length * 19 + 24))}
          />
          <div className="app-file__editor-actions">
            <button
              type="button"
              className="btn-primary"
              data-testid="app-file-save"
              disabled={!dirty || saveFile.isPending}
              onClick={saveDirect}
            >
              {saveFile.isPending ? "Saving…" : "Save"}
            </button>
            <button
              type="button"
              className="btn-ghost"
              data-testid="app-file-revert"
              disabled={!dirty || saveFile.isPending}
              onClick={() => setDraft(text)}
            >
              Revert
            </button>
          </div>
          {saveFile.isError ? (
            <p className="field-error" data-testid="app-file-save-error" role="alert">
              {toUiError(saveFile.error).message}
            </p>
          ) : null}
        </div>
      ) : mode === "agentic" ? (
        <div className="app-file__agentic" data-testid="app-file-agentic">
          {proposal ? (
            <div className="app-file__review" data-testid="app-file-review">
              <p className="muted">
                Review the proposed change, then approve or reject. Nothing is committed yet.
              </p>
              <DiffViewer
                original={proposal.currentText}
                modified={proposal.proposedText}
                language={language}
                testId="app-diff"
                ariaLabel={`Proposed change to ${path}`}
              />
              <div className="app-file__editor-actions">
                <button
                  type="button"
                  className="btn-primary"
                  data-testid="app-edit-approve"
                  disabled={advance.isPending || proposal.proposedText === proposal.currentText}
                  onClick={approve}
                >
                  {advance.isPending ? "Applying…" : "Approve"}
                </button>
                <button
                  type="button"
                  className="btn-ghost"
                  data-testid="app-edit-reject"
                  onClick={reject}
                >
                  Reject
                </button>
              </div>
              {advance.isError ? (
                <p className="field-error" data-testid="app-edit-approve-error" role="alert">
                  {toUiError(advance.error).message}
                </p>
              ) : null}
            </div>
          ) : (
            <>
              <CodeViewer
                value={text}
                language={language}
                testId={`app-file-body-${path}`}
                ariaLabel={`File ${path}`}
                height={Math.min(420, Math.max(160, text.split("\n").length * 19 + 24))}
              />
              <textarea
                className="input"
                data-testid="app-file-edit-instruction"
                placeholder="Describe the change — the agent rewrites this file in-CAS; you review the diff before it commits…"
                rows={2}
                value={instruction}
                disabled={propose.isPending}
                onChange={(e) => setInstruction(e.target.value)}
              />
              <button
                type="button"
                className="btn-primary"
                data-testid="app-file-propose"
                disabled={propose.isPending || instruction.trim().length === 0}
                onClick={runPropose}
              >
                {propose.isPending ? "Proposing…" : "Propose change"}
              </button>
              {propose.isError ? (
                <p className="field-error" data-testid="app-file-propose-error" role="alert">
                  {toUiError(propose.error).message}
                </p>
              ) : null}
            </>
          )}
        </div>
      ) : (
        <CodeViewer
          value={text}
          language={language}
          testId={`app-file-body-${path}`}
          ariaLabel={`File ${path}`}
          height={Math.min(560, Math.max(180, text.split("\n").length * 19 + 24))}
        />
      )}
    </m.div>
  );
}
