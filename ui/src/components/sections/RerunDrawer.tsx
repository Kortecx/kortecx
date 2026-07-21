import { Link, useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import type { StartedRun } from "../../kx/use-invoke";
import { useInvoke } from "../../kx/use-invoke";
import { useProjection } from "../../kx/use-projection";
import { useRecipeForm } from "../../kx/use-recipes";
import { useRunInputs } from "../../kx/use-run-inputs";
import { humanizeHandle } from "../../lib/humanize-handle";
import type { RunRecord } from "../../lib/recent-runs";
import { runAnchor, runViewSearch } from "../../lib/run-anchor";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { RecipeForm } from "../recipes/RecipeForm";

/** The nd_class code for a WORLD_MUTATING Mote (effects re-fire on re-run). */
const ND_WORLD_MUTATING = 3;

/** Parse a run's locally-cached args JSON; `undefined` if absent/malformed. */
function parseLocalArgs(args: string | null | undefined): Record<string, unknown> | undefined {
  if (!args) {
    return undefined;
  }
  try {
    const v = JSON.parse(args) as unknown;
    return v !== null && typeof v === "object" && !Array.isArray(v)
      ? (v as Record<string, unknown>)
      : undefined;
  } catch {
    return undefined;
  }
}

/**
 * "Re-run with changes" (PR-D): a slide-over that pre-fills the run's recipe form
 * with its original args (local cache, else `GetRunInputs` for a durable run),
 * lets the operator edit, and re-invokes. Only the changed sub-DAG recomputes (a
 * free kernel property); an unchanged re-run dedups to the existing result — shown
 * honestly via the no-change banner (don't-fake-gaps). A WORLD_MUTATING prior run
 * gets a confirm-before-fire (effects re-fire). Reuses the `.node-drawer` skeleton
 * + `RecipeForm` (D142.2 — no new pattern). A re-run is just a new `Invoke`.
 */
export function RerunDrawer({
  run,
  anchorMoteId,
  onClose,
}: {
  run: RunRecord;
  /**
   * An override scope anchor for the side-effect probe below. The run-detail route knows
   * the anchor from its `?chain=` (which may be any member Mote — a feed row's event
   * Mote, say — not necessarily one of the `RunHandle` fields a `RunRecord` persists), so
   * it hands it in rather than round-tripping it through a synthesized record.
   */
  anchorMoteId?: string;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const invoke = useInvoke();

  const localArgs = useMemo(() => parseLocalArgs(run.args), [run.args]);
  const hasLocal = Boolean(run.handle && localArgs);
  // Only hit the gateway when we lack (handle + args) locally (the durable-recovery case).
  const inputs = useRunInputs(run.instanceId, !hasLocal);
  // Prior nd_class drives the confirm wording (non-blocking; never fetch-gated). SCOPED
  // to this run: unscoped, the fold is every Mote in the gateway's journal, so ANY
  // world-mutating step anyone ever ran would arm the "this will re-fire side-effects"
  // warning for a read-only re-run — a confirm dialog that cries wolf gets clicked
  // through, which is worse than no confirm at all.
  const projection = useProjection(run.instanceId, {
    scopeMoteId: anchorMoteId ?? runAnchor(run),
  });

  const handle = run.handle ?? inputs.data?.handle ?? undefined;
  const prefill = hasLocal ? localArgs : inputs.data?.args;
  const form = useRecipeForm(handle);

  // The form reads `initial` once (single useState init), so it must not mount
  // until the prefill args are resolved — `handle` can come from `run.handle`
  // BEFORE GetRunInputs returns the durable args. `inputsFailed` is the honest
  // "not captured" case (the fetch ran AND errored), distinct from "still
  // loading / disabled" (which must not claim the args are missing).
  const inputsReady = hasLocal || inputs.isSuccess;
  const inputsFailed = !hasLocal && inputs.isError;

  // `confirm` holds the pending edited args while the operator confirms a
  // side-effecting re-run; `result` holds the no-change outcome.
  const [confirm, setConfirm] = useState<Record<string, unknown> | null>(null);
  const [result, setResult] = useState<StartedRun | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const worldMutating =
    projection.isSuccess &&
    (projection.data?.motes.some((mt) => mt.ndClass === ND_WORLD_MUTATING) ?? false);
  // Unknown when the projection has not resolved — be conservative (confirm). A MISSED
  // scope counts as unknown too: the fold succeeded but this run's Motes are not in it,
  // so the empty set is "we could not look", not "we looked and found nothing
  // side-effecting". Reading it as the latter would silently drop the confirm on exactly
  // the runs we know least about.
  const sideEffectUnknown = !projection.isSuccess || (projection.data?.scopeMissed ?? false);

  function fire(args: Record<string, unknown>): void {
    if (!handle) {
      return;
    }
    setConfirm(null);
    invoke.mutate(
      { handle, args },
      {
        onSuccess: (started) => {
          // One-run-per-journal: an unchanged re-run returns the SAME terminal
          // mote (full dedup) — show the existing result rather than imply a
          // fresh run. Only assert this when we know the prior terminal.
          if (run.terminalMoteId && started.terminalMoteId === run.terminalMoteId) {
            setResult(started);
            return;
          }
          void navigate({
            to: "/workflows/$instanceId",
            params: { instanceId: started.instanceId },
            search: runViewSearch(started),
          });
        },
      },
    );
  }

  function onSubmit(args: Record<string, unknown>): void {
    // Confirm-before-fire only when the prior run may re-fire effects.
    if (worldMutating || sideEffectUnknown) {
      setConfirm(args);
      return;
    }
    fire(args);
  }

  const headline = run.handle ? humanizeHandle(run.handle) : "Re-run with changes";

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close re-run"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay"
        data-testid="rerun-drawer"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the NodeDetailDrawer precedent)
        role="dialog"
        aria-label={`Re-run ${headline} with changes`}
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <strong>Re-run with changes</strong>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        {handle ? (
          <code className="mono muted" title={handle}>
            {handle}
          </code>
        ) : null}

        {/* No-change outcome: the edited args matched the original ⇒ dedup. */}
        {result ? (
          <output className="rerun-banner" data-testid="rerun-no-change">
            <span>Showing existing result — nothing changed (deduped).</span>
            <Link
              to="/workflows/$instanceId"
              params={{ instanceId: result.instanceId }}
              search={runViewSearch(result)}
              className="btnlink"
              data-testid="rerun-view-existing"
              onClick={onClose}
            >
              View result →
            </Link>
          </output>
        ) : null}

        {/* Confirm-before-fire for a side-effecting (or unknown) prior run. */}
        {confirm ? (
          <div className="rerun-confirm" data-testid="rerun-confirm" role="alertdialog">
            <p className="rerun-confirm__warn">
              {worldMutating
                ? "⚠ This re-run will re-execute world-mutating steps with the edited inputs — their side-effects fire again. Unchanged steps are reused."
                : "This may re-run side-effecting steps. Unchanged steps are reused; only the changed sub-DAG recomputes."}
            </p>
            <div className="drawer-actions">
              <button
                type="button"
                className="btnlink"
                data-testid="rerun-confirm-fire"
                disabled={invoke.isPending}
                onClick={() => fire(confirm)}
              >
                {invoke.isPending ? "Running…" : "Re-run"}
              </button>
              <button
                type="button"
                className="linkbtn"
                data-testid="rerun-confirm-cancel"
                onClick={() => setConfirm(null)}
              >
                Cancel
              </button>
            </div>
          </div>
        ) : null}

        {/* Resolve the args, then render the editable form. Order: a failed
            (or handle-less) durable fetch degrades honestly; otherwise wait for
            the prefill to resolve BEFORE mounting the single-init form. */}
        {inputsFailed || (inputsReady && !handle) ? (
          <EmptyState
            title="Inputs not captured for this run"
            detail="This run was started before input-capture, or this gateway doesn't capture inputs. Open the run to view it, or build a new blueprint."
            action={
              <Link
                to="/workflows/$instanceId"
                params={{ instanceId: run.instanceId }}
                search={runViewSearch(run)}
                className="btnlink"
                onClick={onClose}
              >
                Open run →
              </Link>
            }
          />
        ) : !inputsReady ? (
          <EmptyState title="Loading run inputs…" />
        ) : form.isLoading ? (
          <EmptyState title="Loading form…" />
        ) : form.error ? (
          <ErrorNotice error={toUiError(form.error)} onRetry={() => void form.refetch()} />
        ) : form.data && !result ? (
          <RecipeForm
            key={`${handle}:rerun`}
            form={form.data}
            pending={invoke.isPending || confirm !== null}
            onSubmit={onSubmit}
            initial={prefill}
          />
        ) : null}

        {invoke.error ? <ErrorNotice error={toUiError(invoke.error)} /> : null}
      </m.aside>
    </>,
    document.body,
  );
}
