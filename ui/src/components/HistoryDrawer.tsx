import type { BranchVersion } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../kx/errors";
import { useBranchVersions, useRestoreBranch } from "../kx/use-branches";
import { shortHex } from "../lib/format";
import { EmptyState } from "./EmptyState";
import { ErrorNotice } from "./ErrorNotice";

/**
 * The generic point-in-time HISTORY drawer — every recorded version of the
 * branch living at `handle` (`ListBranchVersions`), newest-first, each
 * restorable in place (`RestoreBranch`). Restore APPENDS: nothing is deleted,
 * the restore is itself recorded, so restoring forward again always works.
 *
 * Entity-agnostic by construction (the branch-history sidecar is): what varies
 * per entity rides the props — the title/empty copy, the caller-owned restore
 * GATE (`blockedMessage`; the server refuses either way, the drawer says why up
 * front instead of letting the confirm fail), the cause vocabulary, the
 * post-restore affordance, the cache keys a restore stales, and the testid
 * prefix. `AppHistoryDrawer` wraps this with its historical props; the
 * Workflows def page mounts it directly.
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
  baseline: "The earliest state this history has a record of",
  create: "The branch was created",
  snapshot: "Files were copied in from disk",
  advance: "A recorded item was written or changed",
  restore: "An earlier recorded state was restored",
};

export function HistoryDrawer({
  handle,
  title,
  blockedMessage,
  causeLabels,
  causeTitles,
  afterRestore,
  restoredExtra = "",
  confirmTitle = "Restore this project?",
  confirmSubject = "the project",
  confirmExtra = "",
  emptyState,
  testIdPrefix,
  invalidateOnRestore,
  onClose,
}: {
  handle: string;
  /** The drawer heading (also seeds the aria-label + the not-wired title). */
  title: string;
  /** A caller-owned reason restore is refused up front, or `null` when it isn't
   *  (the gate is the CALLER's — a lock, a live scaffold; the server refuses
   *  either way, this makes the refusal honest before the confirm). */
  blockedMessage: string | null;
  /** Entity-specific overrides merged over the default cause vocabulary. */
  causeLabels?: Record<string, string>;
  causeTitles?: Record<string, string>;
  /** Rendered inside the restored notice, after its sentence (e.g. the hosted
   *  restart affordance). */
  afterRestore?: ReactNode;
  /** Extra sentence appended to the restored notice (entity-specific honesty). */
  restoredExtra?: string;
  /** The confirm dialog heading. */
  confirmTitle?: string;
  /** What restore advances, as prose: "Restore advances {confirmSubject} to …". */
  confirmSubject?: string;
  /** Extra sentence appended to the confirm copy (entity-specific honesty). */
  confirmExtra?: string;
  /** The no-recorded-history empty state. */
  emptyState: { title: string; detail: string };
  /** Prefix for every data-testid (e.g. `app-history` ⇒ `app-history-row-2`). */
  testIdPrefix: string;
  /** Entity-specific query keys a successful restore stales (see
   *  {@link useRestoreBranch}); omitted ⇒ the App branch-manifest default. */
  invalidateOnRestore?: (endpoint: string, handle: string) => ReadonlyArray<readonly unknown[]>;
  onClose: () => void;
}) {
  const { versions, notWired, isLoading, isError, error, refetch } = useBranchVersions(
    handle,
    true,
  );
  const restore = useRestoreBranch(
    invalidateOnRestore === undefined ? {} : { invalidate: invalidateOnRestore },
  );
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

  const blocked = blockedMessage !== null;
  const labels = { ...CAUSE_LABEL, ...causeLabels };
  const titles = { ...CAUSE_TITLE, ...causeTitles };
  const tid = testIdPrefix;

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label={`Close ${title.toLowerCase()}`}
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay"
        data-testid={tid}
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the AppViewPopover precedent)
        role="dialog"
        aria-label={`${title} for ${handle}`}
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <span className="node-drawer__title">{title}</span>
          <div className="node-drawer__head-actions">
            <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
              ✕
            </button>
          </div>
        </div>

        {blocked ? (
          <p className="field-hint" data-testid={`${tid}-blocked`} aria-live="polite">
            {blockedMessage}
          </p>
        ) : null}

        {restoredVersion !== null ? (
          <div className="field-hint" data-testid={`${tid}-restored`} aria-live="polite">
            <p>
              Restored to the state recorded at version {restoredVersion}. The restore is itself
              recorded, so you can restore forward again from this list.
              {restoredExtra}
            </p>
            {afterRestore}
          </div>
        ) : null}

        {notWired ? (
          <EmptyState
            title={`${title} needs a newer server`}
            detail="This gateway predates branch history — upgrade the serve to record and restore versions."
          />
        ) : isLoading ? (
          <EmptyState title="Loading history…" />
        ) : isError ? (
          <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />
        ) : versions === null || versions.length === 0 ? (
          <EmptyState title={emptyState.title} detail={emptyState.detail} />
        ) : (
          <ul className="app-history__list" data-testid={`${tid}-list`}>
            {versions.map((v, i) => {
              // Newest-first: the delta compares against the NEXT (older) row.
              const older = versions[i + 1];
              const delta = older === undefined ? null : v.itemCount - older.itemCount;
              return (
                <li
                  key={v.version}
                  className="app-history__row"
                  data-testid={`${tid}-row-${v.version}`}
                >
                  <div className="app-history__meta">
                    <span
                      className={`app-history__cause app-history__cause--${v.cause}`}
                      title={titles[v.cause] ?? v.cause}
                    >
                      {labels[v.cause] ?? v.cause}
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
                    <span className="app-history__current" data-testid={`${tid}-current`}>
                      current
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="btn-ghost app-history__restore"
                      data-testid={`${tid}-restore-${v.version}`}
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
          <span className="field-error" data-testid={`${tid}-error`} role="alert">
            {toUiError(restore.error).message}
          </span>
        ) : null}
      </m.aside>

      {confirming ? (
        <>
          <button
            type="button"
            className="node-drawer__scrim node-drawer__scrim--overlay node-drawer__scrim--above-drawer"
            aria-label="Cancel restore"
            onClick={() => setConfirming(null)}
          />
          <div className="dialog-center dialog-center--overlay dialog-center--above-drawer">
            <m.div
              className="dialog-card"
              data-testid={`${tid}-confirm`}
              // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; modal semantics via role+aria (the DeleteAppDialog precedent)
              role="dialog"
              aria-label={`Restore ${handle} to version ${confirming.version}`}
              initial={{ y: 12, opacity: 0 }}
              animate={{ y: 0, opacity: 1 }}
              transition={{ type: "spring", stiffness: 420, damping: 34 }}
            >
              <h2 className="dialog-card__title">{confirmTitle}</h2>
              <p className="dialog-card__label">
                Restore advances {confirmSubject} to <strong>version {confirming.version}</strong>
                {confirming.recordedUnixMs > 0
                  ? `, recorded ${new Date(confirming.recordedUnixMs).toLocaleString()}`
                  : ""}
                . Nothing is deleted — the restore is itself recorded, so you can restore forward
                again.
                {confirmExtra}
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
                  data-testid={`${tid}-confirm-restore`}
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
