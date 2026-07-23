/**
 * POC-5d: the single-App IDE — a full-screen workspace (the `.screen` shell, like
 * the run detail) with URL-addressable tabs:
 *  - **Files**: the {@link FileTree} (a collapsible sidebar rail) over the App's CoW
 *    branch manifest + a file pane that VIEWS a file (read-only Monaco), edits it
 *    DIRECTLY (typed Monaco → PutContent → AdvanceBranch), or edits it AGENTICALLY
 *    with a REVIEW/DIFF GATE (propose → diff → approve/reject);
 *  - **Lineage**: a read-only diagram of the blueprint ({@link AppLineageSection});
 *  - **Skills**: attach/detach catalog skills ({@link SkillsRail});
 *  - **MCP Tools** / **Integrations**: the editable capability rails ({@link ToolsRail}
 *    attach/detach; {@link ConnectionsRail} bind/unbind), split.
 * The last three are SCHEDULED-lane only (see {@link HOSTED_TABS}). Under the header sits
 * the per-App {@link AppTriggersStrip} — this App's schedule, and the affordance to add
 * one, where the App is.
 * The header carries the editable App name (left) and top-right actions — Modify
 * (opens the unified agentic-modify {@link AppChatEditDrawer}), Run (opens
 * {@link AppRunDrawer}), Download, and the Lock toggle.
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
import { useApp, useExportAppBundle, useSaveApp } from "../../kx/use-apps";
import { useAdvanceBranch, useEditBranchPropose } from "../../kx/use-branches";
import { useScaffoldStatus } from "../../kx/use-scaffold-app";
import { buildFileTree } from "../../lib/file-tree";
import { inferLanguageFromPath } from "../../lib/monaco/infer-language";
import { loadFlag, persistFlag } from "../../lib/ui-flags";
import { DigestChip } from "../DigestChip";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AppChatEditDrawer } from "../apps/AppChatEditDrawer";
import { AppRunDrawer } from "../apps/AppRunDrawer";
import { AppTriggersStrip } from "../apps/AppTriggersStrip";
import { ConnectionsRail } from "../apps/ConnectionsRail";
import { FileTree } from "../apps/FileTree";
import {
  HostedRestartButton,
  HostedRunButton,
  HostedStatusPill,
  HostedStopButton,
} from "../apps/HostedControls";
import { HostedRunPanel } from "../apps/HostedRunPanel";
import { LockControl } from "../apps/LockControl";
import { SkillsRail } from "../apps/SkillsRail";
import { ToolsRail } from "../apps/ToolsRail";
import { CodeViewer } from "../editor/CodeViewer";
import { DiffViewer } from "../editor/DiffViewer";
import { MonacoMount } from "../editor/MonacoMount";
import { Icon } from "../shell/Icon";
import { AppLineageSection } from "./AppLineageSection";
import { sectionOf } from "./AppsSection";
import { ScaffoldProgress } from "./ScaffoldProgress";

const TABS = ["files", "lineage", "skills", "tools", "integrations"] as const;
export type IdeTab = (typeof TABS)[number];

/**
 * The tabs a HOSTED (experience) App offers.
 *
 * Skills / MCP Tools / Integrations are omitted because the hosted lane provably never
 * reads them: `hostsupervisor.rs` builds its launch plan from the envelope's `hosted`
 * block alone (framework + install/dev/build commands + serve mode + the branch handle),
 * and the code that DOES resolve a tool wish, a skill bundle and a connector — `app_run.rs`
 * — is only reached by `RunApp`, which refuses a hosted App for having no blueprint. So
 * those three rails could only ever write envelope keys nothing will ever read. The tab
 * strip had no kind filter at all, which is the same class of bug D213 already fixed for
 * Run and for Lineage's "Edit structure"; this is the last of it.
 *
 * The TYPE stays the full five (the route validates `?tab=` against it) — a hosted App
 * deep-linked to a lane-less tab falls back to Files rather than 404ing a legal URL.
 */
const HOSTED_TABS: readonly IdeTab[] = ["files", "lineage"];

/** The Files rail's collapsed state is remembered across reloads (like the shell nav). */
const FILES_RAIL_KEY = "kortecx.ui.app-files-rail";

const TAB_LABELS: Record<IdeTab, string> = {
  files: "Files",
  lineage: "Lineage",
  skills: "Skills",
  tools: "MCP Tools",
  integrations: "Integrations",
};

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
  // D213: which lane this App is. Decided the SAME way the catalog decides it
  // (`sectionOf`), because two independent answers is how this page ended up offering
  // scheduled-lane controls — Run, and a Lineage "Edit structure" — on an App with no
  // blueprint, all of which the server refuses. `undefined` kind (loading, or an app
  // from an older server) reads as scheduled, which is the pre-existing behaviour.
  const hosted = summary !== undefined && sectionOf(summary) === "hosted";
  const exportBundle = useExportAppBundle();
  const saveApp = useSaveApp();

  // Editable App name (APP-9): the draft syncs to the loaded envelope name unless the
  // user is mid-edit; committing re-saves the envelope with the new name via SaveApp
  // (which mints a new version). Gated on `locked` (the server refuses a locked write).
  const [nameDraft, setNameDraft] = useState("");
  const [editingName, setEditingName] = useState(false);
  // Re-sync the draft to the loaded name whenever it changes AND the user isn't editing.
  useEffect(() => {
    if (!editingName && summary?.name !== undefined) {
      setNameDraft(summary.name);
    }
  }, [summary?.name, editingName]);

  function commitName(): void {
    setEditingName(false);
    const next = nameDraft.trim();
    if (next === "" || next === summary?.name || !app.data) {
      setNameDraft(summary?.name ?? "");
      return;
    }
    saveApp.mutate({
      handle,
      envelope: { ...(app.data.envelope as Record<string, unknown>), name: next },
    });
  }

  function download(): void {
    exportBundle.mutate(
      { handle },
      {
        onSuccess: (wire) => {
          const url = URL.createObjectURL(new Blob([wire], { type: "application/json" }));
          const a = document.createElement("a");
          a.href = url;
          a.download = `${handle.replace(/\//g, "-")}.kxapp`;
          a.click();
          URL.revokeObjectURL(url);
        },
      },
    );
  }

  const [tabState, setTabState] = useState<IdeTab>("files");
  const [pathState, setPathState] = useState<string | undefined>(undefined);
  const requestedTab = tabProp ?? tabState;
  // Which tabs this lane offers, and the one actually rendered. A hosted App arriving on
  // `?tab=skills` (a legal URL — the route validates the union, not the lane) must not
  // land on a rail its lane never reads, with a tab strip showing nothing pressed.
  const visibleTabs = hosted ? HOSTED_TABS : TABS;
  const tab = visibleTabs.includes(requestedTab) ? requestedTab : "files";
  const selectedPath = pathProp ?? pathState;
  const setTab = (t: IdeTab) => (onTab ? onTab(t) : setTabState(t));
  const setPath = (p: string | undefined) => (onPath ? onPath(p) : setPathState(p));

  const [runOpen, setRunOpen] = useState(false);
  const [chatOpen, setChatOpen] = useState(false);
  const [filesCollapsed, setFilesCollapsed] = useState<boolean>(() => loadFlag(FILES_RAIL_KEY));
  const toggleFiles = () =>
    setFilesCollapsed((v) => {
      const next = !v;
      persistFlag(FILES_RAIL_KEY, next);
      return next;
    });

  const branch = useAppBranch(handle);
  // Is a scaffold WRITING this App's project right now? The chat surface routes here as soon
  // as the App is created, so the live phase — not the (still empty) branch — is what decides
  // whether this page shows progress or a file tree. The query stops polling by itself on a
  // terminal phase, so an App scaffolded long ago costs one read and nothing after it.
  const scaffoldStatus = useScaffoldStatus(tab === "files" ? handle : null, tab === "files");
  const scaffolding =
    scaffoldStatus.data?.phase === "planning" || scaffoldStatus.data?.phase === "writing";
  const items = branch.data?.items ?? [];
  const tree = useMemo(
    () => buildFileTree(items.map((it) => ({ path: it.path, contentRef: it.contentRef }))),
    [items],
  );
  const selected = selectedPath ? (items.find((it) => it.path === selectedPath) ?? null) : null;

  return (
    <section className="screen app-detail" data-testid="app-detail">
      <div className="screen__head">
        <div className="app-detail__title">
          <input
            className="app-detail__name-input"
            data-testid="app-detail-name-input"
            value={nameDraft}
            disabled={locked || app.isLoading}
            aria-label="App name"
            spellCheck={false}
            autoComplete="off"
            title={locked ? "Unlock the App to rename it" : "Rename this App"}
            onChange={(e) => {
              setEditingName(true);
              setNameDraft(e.target.value);
            }}
            onBlur={commitName}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.currentTarget.blur();
              } else if (e.key === "Escape") {
                setEditingName(false);
                setNameDraft(summary?.name ?? "");
                e.currentTarget.blur();
              }
            }}
          />
          {saveApp.isError ? (
            <span className="field-error" data-testid="app-detail-name-error" role="alert">
              {toUiError(saveApp.error).message}
            </span>
          ) : null}
        </div>
        <div className="screen__head-actions">
          <button
            type="button"
            className="iconbtn"
            data-testid="app-detail-chat"
            title="Modify this App with the agent (multi-file diff with rollback)"
            aria-label="Modify app"
            onClick={() => setChatOpen(true)}
          >
            <Icon name="chat" size={18} />
          </button>
          {hosted ? (
            // A hosted App has no blueprint, so RunApp refuses it by construction
            // (app_run.rs: "this is a hosted (experience) app with no blueprint"). Offer
            // the controls that CAN fire instead: start-and-open, stop, restart clean.
            <>
              <HostedStatusPill handle={handle} variant="detail" />
              <HostedRunButton handle={handle} variant="detail" />
              <HostedStopButton handle={handle} />
              <HostedRestartButton handle={handle} />
            </>
          ) : (
            <button
              type="button"
              className="iconbtn"
              data-testid="app-detail-run"
              title="Run this App"
              aria-label="Run"
              onClick={() => setRunOpen(true)}
            >
              <Icon name="play" size={18} />
            </button>
          )}
          <button
            type="button"
            className="iconbtn"
            data-testid="app-detail-download"
            disabled={exportBundle.isPending}
            title="Download a portable .kxapp bundle (envelope + content closure)"
            aria-label="Download bundle"
            onClick={download}
          >
            <Icon name="download" size={18} />
          </button>
          <LockControl handle={handle} locked={locked} />
        </div>
      </div>

      {/* The App's own schedule, where the App is. Scheduled lane only: a trigger fires an
          App through RunApp, which refuses a hosted App for having no blueprint — a
          Schedule button on a hosted App would register a trigger that can only ever
          fail. */}
      {hosted ? null : <AppTriggersStrip handle={handle} />}

      <fieldset className="view-toggle" aria-label="App view" data-testid="app-detail-tabs">
        {visibleTabs.map((t) => (
          <button
            key={t}
            type="button"
            aria-pressed={tab === t}
            data-testid={`app-tab-${t}`}
            onClick={() => setTab(t)}
          >
            {TAB_LABELS[t]}
          </button>
        ))}
      </fieldset>

      {tab === "lineage" ? (
        // Lineage survives in both lanes, because the question ("what is this App doing?")
        // survives — it is just answered by the server surface rather than by a blueprint
        // diagram the hosted lane does not have.
        hosted ? (
          <HostedRunPanel handle={handle} />
        ) : (
          <AppLineageSection handle={handle} />
        )
      ) : tab === "skills" ? (
        app.data ? (
          <SkillsRail handle={handle} envelope={app.data.envelope} locked={locked} />
        ) : (
          <EmptyState title="Loading skills…" />
        )
      ) : tab === "tools" ? (
        app.data ? (
          <ToolsRail handle={handle} envelope={app.data.envelope} locked={locked} />
        ) : (
          <EmptyState title="Loading tools…" />
        )
      ) : tab === "integrations" ? (
        app.data ? (
          <ConnectionsRail handle={handle} envelope={app.data.envelope} locked={locked} />
        ) : (
          <EmptyState title="Loading integrations…" />
        )
      ) : scaffolding ? (
        // The scaffold runs SERVER-side and the chat surface routes here the moment the App is
        // created, so this page — not a form that has already closed — is where an author
        // watches their project get written. Terminal phases fall through to the file tree
        // below, which is the real artifact once there is one.
        <ScaffoldProgress branchHandle={handle} appHandle={handle} />
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
        <div
          className="app-detail__panes"
          data-testid="app-detail-panes"
          data-collapsed={filesCollapsed ? "true" : "false"}
        >
          <aside className="app-detail__tree" data-testid="app-files-sidebar">
            <div className="app-detail__tree-head">
              <span className="app-detail__tree-title">Files</span>
              <button
                type="button"
                className="iconbtn iconbtn--sm app-detail__tree-toggle"
                data-testid="app-files-collapse"
                aria-label={filesCollapsed ? "Show files" : "Hide files"}
                aria-expanded={!filesCollapsed}
                title={filesCollapsed ? "Show files" : "Hide files"}
                onClick={toggleFiles}
              >
                <span className="app-detail__chevron" aria-hidden="true">
                  <Icon name="chevron-right" size={16} />
                </span>
              </button>
            </div>
            {filesCollapsed ? null : (
              <FileTree
                nodes={tree}
                selectedPath={selected?.path ?? null}
                onSelect={(path) => setPath(path)}
              />
            )}
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

      {/* Never for hosted: the drawer's RunPreflight would print "✓ Ready — a model is
          served and this App's capabilities are in policy" and then the run would be
          refused for having no blueprint. Not mounting it is the honest fix; a preflight
          that green-lights a structurally impossible run is worse than no preflight. */}
      {runOpen && !hosted ? (
        <AppRunDrawer handle={handle} onClose={() => setRunOpen(false)} />
      ) : null}
      {chatOpen ? (
        <AppChatEditDrawer handle={handle} locked={locked} onClose={() => setChatOpen(false)} />
      ) : null}
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
        <code className="mono app-file__path" title={path}>
          {path}
        </code>
        {/* The selected file's content ref. It used to sit on every tree ROW, where a
            180px rail left the filename ~6 characters; here there is width for both,
            and only the file you are actually looking at needs its hash. */}
        {contentRef ? <DigestChip hex={contentRef} label={path} /> : null}
        {locked ? (
          <span className="muted app-file__locked" data-testid="app-locked-notice" role="note">
            This App is locked — edits are refused. Unlock it from this App's header to edit.
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
