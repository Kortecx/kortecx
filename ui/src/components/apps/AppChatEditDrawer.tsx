/**
 * item6 — the per-App unified AGENTIC MODIFY drawer, reached from the Modify action in
 * the App header (next to run / download / lock). ONE surface: the earlier converse-vs-edit
 * split is gone. Describe a change in high-level natural language, pick the project files it
 * touches, and the agent proposes a COHERENT MULTI-ARTIFACT diff you review as a whole,
 * approve, and can ROLL BACK — all client-side over the SHIPPED propose→advance path (no new
 * RPC, no proto change, no server recipe change):
 *
 *  - PROPOSE: run the shipped `kx/recipes/react-edit` loop once per selected file, attaching
 *    every selected file's current body as context (`context_refs` is already `repeated
 *    string`), so the per-file rewrites stay consistent with one another.
 *  - REVIEW: one review gate over every proposed file, each a current→proposed diff.
 *  - APPROVE: N sequential `AdvanceBranch` calls re-point each path to its proposed ref.
 *  - ROLLBACK: each file's PRIOR `content_ref` is captured before the advance and its CAS
 *    blob is still present, so rollback is just `AdvanceBranch(handle, path, priorRef)` per
 *    file — no history RPC.
 *
 * A LOCKED App refuses in-CAS edits at the server (LOCKED_BRANCH); the UI pre-gates so the
 * modify affordance is replaced by an honest notice.
 */

import { m } from "framer-motion";
import { useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import { useAppBranch } from "../../kx/use-app-files";
import { useAdvanceBranch, useEditBranchPropose } from "../../kx/use-branches";
import { inferLanguageFromPath } from "../../lib/monaco/infer-language";
import { ErrorNotice } from "../ErrorNotice";
import { DiffViewer } from "../editor/DiffViewer";

/** One file's proposed change + the ref it pointed at BEFORE approve (the rollback target). */
interface ProposalRow {
  readonly path: string;
  readonly priorRef: string;
  readonly resultRef: string;
  readonly currentText: string;
  readonly proposedText: string;
}

type Phase = "compose" | "review" | "applied";

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

  const [instruction, setInstruction] = useState("");
  const [selected, setSelected] = useState<readonly string[]>([]);
  const [phase, setPhase] = useState<Phase>("compose");
  const [proposals, setProposals] = useState<readonly ProposalRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  // Only files whose proposed body actually differs are real changes — an unchanged
  // rewrite is a no-op we neither review, apply, nor roll back.
  const changed = proposals.filter((p) => p.proposedText !== p.currentText);

  function toggle(path: string): void {
    setSelected((prev) => (prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path]));
  }

  function resetToCompose(): void {
    propose.reset();
    advance.reset();
    setProposals([]);
    setPhase("compose");
    setError(null);
  }

  function startOver(): void {
    setInstruction("");
    setSelected([]);
    resetToCompose();
  }

  async function runPropose(): Promise<void> {
    const instr = instruction.trim();
    const targets = files.filter((f) => selected.includes(f.path));
    if (instr === "" || targets.length === 0 || busy) {
      return;
    }
    setBusy(true);
    setError(null);
    // Every selected file rides along as coherence context on each per-file rewrite.
    const contextPaths = targets.map((f) => f.path);
    try {
      const rows: ProposalRow[] = [];
      for (const f of targets) {
        const res = await propose.mutateAsync({
          handle,
          path: f.path,
          instruction: instr,
          contextPaths,
        });
        rows.push({
          path: f.path,
          priorRef: f.contentRef, // the still-present CAS blob to roll back to
          resultRef: res.resultRef,
          currentText: res.currentText,
          proposedText: res.proposedText,
        });
      }
      setProposals(rows);
      setPhase("review");
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  async function approve(): Promise<void> {
    if (busy || changed.length === 0) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // N sequential AdvanceBranch calls — the whole multi-artifact diff commits together.
      for (const p of changed) {
        await advance.mutateAsync({ handle, path: p.path, contentRef: p.resultRef });
      }
      await branch.refetch();
      setPhase("applied");
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  async function rollback(): Promise<void> {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // Re-advance each path to its prior ref (the blob is still in the content store).
      for (const p of changed) {
        await advance.mutateAsync({ handle, path: p.path, contentRef: p.priorRef });
      }
      await branch.refetch();
      startOver();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  const proposeDisabled = instruction.trim() === "" || selected.length === 0 || busy;

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close modify drawer"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay app-chat-drawer"
        data-testid="app-chat-drawer"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the NodeDetailDrawer precedent)
        role="dialog"
        aria-label={`Modify ${handle}`}
        initial={{ x: 28, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <strong>Modify app</strong>
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

        {locked ? (
          <p
            className="muted app-chat-drawer__locked"
            data-testid="app-chat-edit-locked"
            role="note"
          >
            This App is locked — the agent can't modify project files. Unlock it (the header lock)
            to describe a change.
          </p>
        ) : (
          <section className="app-chat-drawer__edit" data-testid="app-chat-edit">
            {phase === "compose" ? (
              <>
                <h3 className="app-chat-drawer__edit-title">Modify with the agent</h3>
                <p className="muted">
                  Describe the change in plain language and pick the files it touches — the agent
                  rewrites each in-CAS and you review one combined diff before anything commits.
                  Nothing is applied until you approve, and you can roll the whole change back.
                </p>
                <textarea
                  className="input"
                  data-testid="app-edit-instruction"
                  rows={2}
                  placeholder="e.g. rename the widget to Gadget across the project and update its docs"
                  value={instruction}
                  disabled={busy}
                  onChange={(e) => setInstruction(e.target.value)}
                />
                <fieldset className="app-chat-drawer__field" data-testid="app-edit-files">
                  <legend className="muted">Files in scope</legend>
                  {files.length === 0 ? (
                    <p className="muted" data-testid="app-edit-no-files">
                      This App has no project files yet.
                    </p>
                  ) : (
                    files.map((f) => (
                      <label key={f.path} className="app-chat-drawer__file">
                        <input
                          type="checkbox"
                          data-testid={`app-edit-file-${f.path}`}
                          checked={selected.includes(f.path)}
                          disabled={busy}
                          onChange={() => toggle(f.path)}
                        />
                        <span>{f.path}</span>
                      </label>
                    ))
                  )}
                </fieldset>
                <button
                  type="button"
                  className="btn-primary"
                  data-testid="app-edit-propose"
                  disabled={proposeDisabled}
                  onClick={() => void runPropose()}
                >
                  {busy
                    ? "Proposing…"
                    : `Propose ${selected.length > 1 ? `${selected.length} changes` : "change"}`}
                </button>
                {error ? (
                  <ErrorNotice error={toUiError(error)} onRetry={() => setError(null)} />
                ) : null}
              </>
            ) : null}

            {phase === "review" ? (
              <div className="app-chat-drawer__review" data-testid="app-edit-review">
                <p className="muted">
                  {changed.length === 0
                    ? "The agent proposed no changes to the selected files."
                    : `Review ${changed.length} proposed file change${
                        changed.length > 1 ? "s" : ""
                      }, then approve or reject.`}
                </p>
                {changed.map((p) => (
                  <div
                    key={p.path}
                    className="app-chat-drawer__review-file"
                    data-testid={`app-edit-review-${p.path}`}
                  >
                    <strong className="app-chat-drawer__review-path">{p.path}</strong>
                    <DiffViewer
                      original={p.currentText}
                      modified={p.proposedText}
                      language={inferLanguageFromPath(p.path)}
                      testId={`app-edit-diff-${p.path}`}
                      ariaLabel={`Proposed change to ${p.path}`}
                    />
                  </div>
                ))}
                <div className="app-chat-drawer__review-actions">
                  <button
                    type="button"
                    className="btn-primary"
                    data-testid="app-edit-approve"
                    disabled={busy || changed.length === 0}
                    onClick={() => void approve()}
                  >
                    {busy
                      ? "Applying…"
                      : `Approve ${changed.length > 1 ? `${changed.length} changes` : "change"}`}
                  </button>
                  <button
                    type="button"
                    className="btn-ghost"
                    data-testid="app-edit-reject"
                    disabled={busy}
                    onClick={resetToCompose}
                  >
                    Reject
                  </button>
                </div>
                {error ? (
                  <ErrorNotice error={toUiError(error)} onRetry={() => setError(null)} />
                ) : null}
              </div>
            ) : null}

            {phase === "applied" ? (
              <div className="app-chat-drawer__review" data-testid="app-edit-applied">
                <p className="muted">
                  Applied {changed.length} file change{changed.length > 1 ? "s" : ""}. Roll back to
                  restore the previous contents (the prior versions are still in the content store).
                </p>
                <ul className="app-chat-drawer__applied-list">
                  {changed.map((p) => (
                    <li key={p.path} data-testid={`app-edit-applied-${p.path}`}>
                      {p.path}
                    </li>
                  ))}
                </ul>
                <div className="app-chat-drawer__review-actions">
                  <button
                    type="button"
                    className="btn-ghost"
                    data-testid="app-edit-rollback"
                    disabled={busy}
                    onClick={() => void rollback()}
                  >
                    {busy ? "Rolling back…" : "Roll back"}
                  </button>
                  <button
                    type="button"
                    className="btn-primary"
                    data-testid="app-edit-done"
                    disabled={busy}
                    onClick={startOver}
                  >
                    Done
                  </button>
                </div>
                {error ? (
                  <ErrorNotice error={toUiError(error)} onRetry={() => setError(null)} />
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
