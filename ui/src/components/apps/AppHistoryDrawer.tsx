import type { BranchVersion } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import { useBranchVersions, useRestoreBranch } from "../../kx/use-branches";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { HostedRestartButton } from "./HostedControls";

/**
 * The App project HISTORY drawer — every recorded point-in-time of the project
 * branch (`ListBranchVersions`), newest-first, each restorable in place
 * (`RestoreBranch`). Restore APPENDS: nothing is deleted, the restore is itself
 * recorded, so restoring forward again always works. A running hosted app keeps
 * serving its pre-restore files until it is restarted — the success state says
 * so and offers the restart.
 *
 * Pre-gates mirror the Modify drawer's honesty: a locked App and a live
 * scaffold both refuse restore SERVER-side, so the drawer says why up front
 * instead of letting the confirm fail.
 */

/**
 * What each recorded cause MEANS to the person reading it. The wire values are
 * the store's own vocabulary (`advance` is the branch-store verb for a write);
 * rendering them verbatim asked the reader to know it. `create` and `restore`
 * happen to read fine either way — they are mapped anyway, so the drawer never
 * shows a raw enum for one cause and prose for the next.
 */
const CAUSE_LABEL: Record<string, string> = {
  baseline: "Starting point",
  create: "Created",
  snapshot: "Snapshot",
  advance: "Edited",
  restore: "Restored",
};

const CAUSE_TITLE: Record<string, string> = {
  baseline: "The earliest state this project has a record of",
  create: "The project branch was created",
  snapshot: "Files were copied in from disk",
  advance: "A file in the project was written or changed",
  restore: "An earlier recorded state was restored onto the project",
};

export function AppHistoryDrawer({
  handle,
  locked,
  scaffolding,
  hosted,
  onClose,
}: {
  handle: string;
  locked: boolean;
  scaffolding: boolean;
  hosted: boolean;
  onClose: () => void;
}) {
  const { versions, notWired, isLoading, isError, error, refetch } = useBranchVersions(
    handle,
    true,
  );
  const restore = useRestoreBranch();
  const [confirming, setConfirming] = useState<BranchVersion | null>(null);
  const [restoredVersion, setRestoredVersion] = useState<number | null>(null);
  const cancelRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        if (confirming) {
          setConfirming(null);
        } else {
          onClose();
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, confirming]);

  // Focus the SAFE button when the confirm opens (the DeleteAppDialog idiom).
  useEffect(() => {
    if (confirming) {
      cancelRef.current?.focus();
    }
  }, [confirming]);

  const blocked = locked || scaffolding;

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close project history"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay"
        data-testid="app-history"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the AppViewPopover precedent)
        role="dialog"
        aria-label={`Project history for ${handle}`}
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <span className="node-drawer__title">Project history</span>
          <div className="node-drawer__head-actions">
            <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
              ✕
            </button>
          </div>
        </div>

        {blocked ? (
          <p className="field-hint" data-testid="app-history-blocked" aria-live="polite">
            {locked
              ? "This App is locked — restore is refused while the lock is held. Unlock the App to restore."
              : "A scaffold is writing this project — restore is refused until it finishes."}
          </p>
        ) : null}

        {restoredVersion !== null ? (
          <div className="field-hint" data-testid="app-history-restored" aria-live="polite">
            <p>
              Restored to the state recorded at version {restoredVersion}. The restore is itself
              recorded, so you can restore forward again from this list.
              {hosted
                ? " This hosted app keeps serving its previous files until you restart it."
                : ""}
            </p>
            {hosted ? <HostedRestartButton handle={handle} /> : null}
          </div>
        ) : null}

        {notWired ? (
          <EmptyState
            title="Project history needs a newer server"
            detail="This gateway predates branch history — upgrade the serve to record and restore project versions."
          />
        ) : isLoading ? (
          <EmptyState title="Loading history…" />
        ) : isError ? (
          <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />
        ) : versions === null || versions.length === 0 ? (
          <EmptyState
            title="No recorded history yet"
            detail="This App has no project branch yet. Once its project exists, every change to it is recorded here and any recorded state can be restored."
          />
        ) : (
          <ul className="app-history__list" data-testid="app-history-list">
            {versions.map((v, i) => {
              // Newest-first: the delta compares against the NEXT (older) row.
              const older = versions[i + 1];
              const delta = older === undefined ? null : v.itemCount - older.itemCount;
              return (
                <li
                  key={v.version}
                  className="app-history__row"
                  data-testid={`app-history-row-${v.version}`}
                >
                  <div className="app-history__meta">
                    <span
                      className={`app-history__cause app-history__cause--${v.cause}`}
                      title={CAUSE_TITLE[v.cause] ?? v.cause}
                    >
                      {CAUSE_LABEL[v.cause] ?? v.cause}
                    </span>
                    <span className="app-history__when">
                      {v.recordedUnixMs > 0
                        ? new Date(v.recordedUnixMs).toLocaleString()
                        : "unknown time"}
                    </span>
                  </div>
                  <div className="app-history__detail">
                    <span>
                      v{v.version} · {v.itemCount} file{v.itemCount === 1 ? "" : "s"}
                      {delta !== null && delta !== 0 ? (
                        <span className="app-history__delta">
                          {" "}
                          ({delta > 0 ? "+" : ""}
                          {delta})
                        </span>
                      ) : null}
                    </span>
                    <code className="mono app-history__ref" title={v.branchRef}>
                      {shortHex(v.branchRef)}
                    </code>
                  </div>
                  {i === 0 ? (
                    <span className="app-history__current" data-testid="app-history-current">
                      current
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="btn-ghost app-history__restore"
                      data-testid={`app-history-restore-${v.version}`}
                      disabled={blocked || restore.isPending}
                      onClick={() => {
                        setRestoredVersion(null);
                        setConfirming(v);
                      }}
                    >
                      Restore
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        )}

        {restore.isError ? (
          <span className="field-error" data-testid="app-history-error" role="alert">
            {toUiError(restore.error).message}
          </span>
        ) : null}
      </m.aside>

      {confirming ? (
        <>
          <button
            type="button"
            className="node-drawer__scrim node-drawer__scrim--overlay"
            aria-label="Cancel restore"
            onClick={() => setConfirming(null)}
          />
          <div className="dialog-center dialog-center--overlay">
            <m.div
              className="dialog-card"
              data-testid="app-history-confirm"
              // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; modal semantics via role+aria (the DeleteAppDialog precedent)
              role="dialog"
              aria-label={`Restore ${handle} to version ${confirming.version}`}
              initial={{ y: 12, opacity: 0 }}
              animate={{ y: 0, opacity: 1 }}
              transition={{ type: "spring", stiffness: 420, damping: 34 }}
            >
              <h2 className="dialog-card__title">Restore this project?</h2>
              <p className="dialog-card__label">
                Restore advances the project to <strong>version {confirming.version}</strong>
                {confirming.recordedUnixMs > 0
                  ? `, recorded ${new Date(confirming.recordedUnixMs).toLocaleString()}`
                  : ""}
                . Nothing is deleted — the restore is itself recorded, so you can restore forward
                again.
                {hosted
                  ? " A running hosted app keeps serving its current files until you restart it."
                  : ""}
              </p>
              <div className="dialog-card__actions">
                <button
                  ref={cancelRef}
                  type="button"
                  className="btn-ghost"
                  onClick={() => setConfirming(null)}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="btn-primary"
                  data-testid="app-history-confirm-restore"
                  disabled={restore.isPending}
                  onClick={() => {
                    const version = confirming.version;
                    restore.mutate(
                      { handle, version },
                      {
                        onSuccess: () => {
                          setConfirming(null);
                          setRestoredVersion(version);
                        },
                        onError: () => setConfirming(null),
                      },
                    );
                  }}
                >
                  {restore.isPending ? "Restoring…" : "Restore"}
                </button>
              </div>
            </m.div>
          </div>
        </>
      ) : null}
    </>,
    document.body,
  );
}
