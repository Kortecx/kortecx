/**
 * Snapshot host files into a branch (`SnapshotInto`, D155) — or create/fork an
 * empty branch (`CreateBranch`). Paths are read SERVER-side from the operator's
 * confined `KX_SERVE_FS_ROOT` (the host is never written); the server derives
 * `branchRef` (SN-8). Mirrors `NewContextBundleForm` (GlowCard + chip/inline
 * controls, never a controlled `<select>`). `--parent` forks a point-in-time CoW
 * sub-branch.
 */

import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useCreateBranch, useSnapshotInto } from "../../kx/use-branches";
import { GlowCard } from "../ds/GlowCard";

export function NewBranchForm() {
  const [handle, setHandle] = useState("");
  const [parent, setParent] = useState("");
  const [description, setDescription] = useState("");
  const [pathDraft, setPathDraft] = useState("");
  const [paths, setPaths] = useState<string[]>([]);
  const [localError, setLocalError] = useState<string | null>(null);

  const snapshot = useSnapshotInto();
  const create = useCreateBranch();

  const handleOk = handle.trim().length > 0;
  const canSnapshot = handleOk && paths.length > 0 && !snapshot.isPending;
  const canCreate = handleOk && !create.isPending;

  function addPath(): void {
    const p = pathDraft.trim();
    if (p.length === 0) {
      return;
    }
    setLocalError(null);
    setPaths((prev) => (prev.includes(p) ? prev : [...prev, p]));
    setPathDraft("");
  }

  function removePath(idx: number): void {
    setPaths((prev) => prev.filter((_, i) => i !== idx));
  }

  function reset(): void {
    setHandle("");
    setParent("");
    setDescription("");
    setPaths([]);
    setPathDraft("");
  }

  function onSnapshot(e: FormEvent): void {
    e.preventDefault();
    if (!canSnapshot) {
      return;
    }
    snapshot.mutate(
      {
        handle: handle.trim(),
        paths,
        parent: parent.trim() || undefined,
        description: description.trim() || undefined,
      },
      { onSuccess: () => reset() },
    );
  }

  function onCreate(): void {
    if (!canCreate) {
      return;
    }
    create.mutate(
      {
        handle: handle.trim(),
        parent: parent.trim() || undefined,
        description: description.trim() || undefined,
      },
      { onSuccess: () => reset() },
    );
  }

  const snapErr = snapshot.error ? toUiError(snapshot.error) : null;
  const createErr = create.error ? toUiError(create.error) : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="branch-new">
      <h2>New branch</h2>
      <p className="muted">
        Snapshot operator-approved host files into a content-addressed branch, or create/fork an
        empty one. Files are read server-side from the confined read root (
        <code>KX_SERVE_FS_ROOT</code>) — the host is never written. A parent handle forks a
        point-in-time copy-on-write sub-branch.
      </p>
      <form onSubmit={onSnapshot} className="register-tool-form">
        <div className="register-tool-form__row">
          <input
            type="text"
            data-testid="branch-handle"
            placeholder="handle (e.g. team/workspace/main)"
            value={handle}
            onChange={(e) => setHandle(e.target.value)}
            aria-label="Branch handle"
          />
          <input
            type="text"
            data-testid="branch-parent"
            placeholder="parent handle (optional — fork)"
            value={parent}
            onChange={(e) => setParent(e.target.value)}
            aria-label="Parent branch handle"
          />
        </div>
        <input
          type="text"
          data-testid="branch-description"
          placeholder="description (optional)"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          aria-label="Branch description"
        />

        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Paths to snapshot</legend>
          <div className="context-bundle-form__ref">
            <input
              type="text"
              className="mono"
              data-testid="branch-path-draft"
              placeholder="path under the read root (e.g. src/lib.rs)"
              value={pathDraft}
              onChange={(e) => setPathDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addPath();
                }
              }}
              aria-label="Snapshot path"
            />
            <button type="button" className="chip" data-testid="branch-add-path" onClick={addPath}>
              <span className="chip__label">+ Add path</span>
            </button>
          </div>
          {paths.length > 0 ? (
            <ul className="context-bundle__items" data-testid="branch-staged-paths">
              {paths.map((p, idx) => (
                <li key={p} className="context-bundle__item">
                  <span className="context-bundle__item-name mono">{p}</span>
                  <button
                    type="button"
                    className="btn-ghost"
                    data-testid={`branch-staged-remove-${idx}`}
                    aria-label={`Remove ${p}`}
                    onClick={() => removePath(idx)}
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted">
              No paths yet — add a path to snapshot, or create an empty branch.
            </p>
          )}
        </fieldset>

        <div className="register-tool-form__row">
          <button type="submit" data-testid="branch-snapshot-submit" disabled={!canSnapshot}>
            {snapshot.isPending ? "Snapshotting…" : "Snapshot paths"}
          </button>
          <button
            type="button"
            className="btn-ghost"
            data-testid="branch-create-submit"
            disabled={!canCreate}
            onClick={onCreate}
          >
            {create.isPending ? "Creating…" : "Create branch"}
          </button>
        </div>
      </form>

      {localError ? (
        <p className="field-error" data-testid="branch-local-error" role="alert">
          {localError}
        </p>
      ) : null}
      {snapErr ? (
        <p className="field-error" data-testid="branch-snapshot-error" role="alert">
          {snapErr.message}
        </p>
      ) : null}
      {createErr ? (
        <p className="field-error" data-testid="branch-create-error" role="alert">
          {createErr.message}
        </p>
      ) : null}
      {snapshot.isSuccess ? (
        <p className="register-tool__result" data-testid="branch-snapshot-result">
          Snapshotted <code className="mono">{snapshot.data?.handle}</code> —{" "}
          {snapshot.data?.ingested} file(s) read{snapshot.data?.deduplicated ? " (unchanged)" : ""}
        </p>
      ) : null}
      {create.isSuccess ? (
        <p className="register-tool__result" data-testid="branch-create-result">
          Created <code className="mono">{create.data?.handle}</code>
          {create.data?.deduplicated ? " (unchanged — identical manifest)" : ""}
        </p>
      ) : null}
    </GlowCard>
  );
}
