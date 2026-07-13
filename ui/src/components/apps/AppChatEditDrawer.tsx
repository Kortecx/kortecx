/**
 * R3 — the per-App "Chat & Edit" slide-over: the agentic MODIFY surface, reached from a
 * Chat action in the App header (next to run / download / lock). Two things, scoped to
 * ONE App:
 *  - CONVERSE: the embedded {@link AppChat} (understand the App, ask questions).
 *  - MODIFY: describe a change to a project file → the agent proposes a rewrite → you
 *    REVIEW the diff → approve/reject. This is the SHIPPED propose→diff→approve gate
 *    (`EditBranchPropose` → `AdvanceBranch`) — no new RPC; the same honest, single-user
 *    agentic loop the Files tab uses, surfaced conversationally and scoped to the App
 *    (the App handle is the "pointer" the runtime edits against).
 *
 * A LOCKED App refuses in-CAS edits at the server (LOCKED_BRANCH); the UI pre-gates so
 * the edit affordance is hidden — converse only.
 */

import { m } from "framer-motion";
import { useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import { useAppBranch } from "../../kx/use-app-files";
import { useAdvanceBranch, useEditBranchPropose } from "../../kx/use-branches";
import { inferLanguageFromPath } from "../../lib/monaco/infer-language";
import { ErrorNotice } from "../ErrorNotice";
import { AppChat } from "../chat/AppChat";
import { DiffViewer } from "../editor/DiffViewer";

export function AppChatEditDrawer({
  handle,
  locked,
  onClose,
}: {
  handle: string;
  locked: boolean;
  onClose: () => void;
}) {
  const branch = useAppBranch(handle);
  const files = branch.data?.items ?? [];
  const propose = useEditBranchPropose();
  const advance = useAdvanceBranch();
  const [targetPath, setTargetPath] = useState("");
  const [instruction, setInstruction] = useState("");
  const proposal = propose.data ?? null;
  const language = targetPath ? inferLanguageFromPath(targetPath) : "plaintext";

  function runPropose(): void {
    const instr = instruction.trim();
    if (targetPath === "" || instr === "") {
      return;
    }
    propose.mutate({ handle, path: targetPath, instruction: instr });
  }
  function approve(): void {
    if (!proposal) {
      return;
    }
    advance.mutate(
      { handle, path: targetPath, contentRef: proposal.resultRef },
      {
        onSuccess: () => {
          propose.reset();
          setInstruction("");
          void branch.refetch();
        },
      },
    );
  }

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close chat"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay app-chat-drawer"
        data-testid="app-chat-drawer"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the NodeDetailDrawer precedent)
        role="dialog"
        aria-label={`Chat and edit ${handle}`}
        initial={{ x: 28, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <strong>Chat &amp; edit</strong>
          <button
            type="button"
            className="linkbtn"
            data-testid="app-chat-drawer-close"
            onClick={onClose}
            aria-label="Close"
          >
            ✕
          </button>
        </div>

        <div className="app-chat-drawer__chat">
          <AppChat recipeHandle={handle} agentMode />
        </div>

        {locked ? (
          <p
            className="muted app-chat-drawer__locked"
            data-testid="app-chat-edit-locked"
            role="note"
          >
            This App is locked — the chat is read-only. Unlock it (the header lock) to have the
            agent modify project files.
          </p>
        ) : (
          <section className="app-chat-drawer__edit" data-testid="app-chat-edit">
            <h3 className="app-chat-drawer__edit-title">Modify a file with the agent</h3>
            <p className="muted">
              Pick a file, describe the change — the agent rewrites it in-CAS and you review the
              diff before it commits (nothing is applied until you approve).
            </p>
            <label className="app-chat-drawer__field">
              <span className="muted">File</span>
              <select
                className="input"
                data-testid="app-edit-target"
                value={targetPath}
                onChange={(e) => {
                  setTargetPath(e.target.value);
                  propose.reset();
                }}
              >
                <option value="">Select a file…</option>
                {files.map((f) => (
                  <option key={f.path} value={f.path}>
                    {f.path}
                  </option>
                ))}
              </select>
            </label>
            <textarea
              className="input"
              data-testid="app-edit-instruction"
              rows={2}
              placeholder="e.g. add input validation and a docstring"
              value={instruction}
              disabled={propose.isPending}
              onChange={(e) => setInstruction(e.target.value)}
            />
            <button
              type="button"
              className="btn-primary"
              data-testid="app-edit-propose"
              disabled={targetPath === "" || instruction.trim() === "" || propose.isPending}
              onClick={runPropose}
            >
              {propose.isPending ? "Proposing…" : "Propose change"}
            </button>
            {propose.isError ? (
              <ErrorNotice error={toUiError(propose.error)} onRetry={() => propose.reset()} />
            ) : null}

            {proposal ? (
              <div className="app-chat-drawer__review" data-testid="app-edit-review">
                <p className="muted">Review the proposed change, then approve or reject.</p>
                <DiffViewer
                  original={proposal.currentText}
                  modified={proposal.proposedText}
                  language={language}
                  testId="app-edit-diff"
                  ariaLabel={`Proposed change to ${targetPath}`}
                />
                <div className="app-chat-drawer__review-actions">
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
                    onClick={() => propose.reset()}
                  >
                    Reject
                  </button>
                </div>
                {advance.isError ? (
                  <ErrorNotice error={toUiError(advance.error)} onRetry={() => advance.reset()} />
                ) : null}
              </div>
            ) : null}
          </section>
        )}
      </m.aside>
    </>,
    document.body,
  );
}
