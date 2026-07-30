/**
 * The stored-Workflow DEFINITION page (`/workflows/def/$handle`) — the summary
 * (name · description · steps · lifecycle) with the actions a durable workflow
 * carries: Run (server-side `RunWorkflow` → the live run view), History (the
 * generic point-in-time drawer over the definition branch), Edit (the create
 * screen seeded by `?handle=`), Delete (a confirm dialog — the cascade names
 * what goes AND what stays).
 *
 * The per-step list is derived with the Lineage view-model (`lineageStepViews`
 * — pure, honesty rules verbatim: a tool_contract is a WISH the card says
 * "requests" about; a budget renders only when explicitly authored; an empty
 * model_id is a run-time binding, said rather than blanked).
 *
 * A DRAFT swaps Run for "Finish draft" (→ the create screen): a draft can't
 * honestly offer Run — the server refuses trigger registration for it and the
 * author parked it unfinished on purpose.
 */

import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import { queryKeys } from "../../kx/query-keys";
import { useDeleteWorkflow, useRunWorkflow, useWorkflow } from "../../kx/use-workflows";
import { readModelRoute } from "../../lib/app-envelope";
import { runViewSearch } from "../../lib/run-anchor";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { HistoryDrawer } from "../HistoryDrawer";
import { appBlueprintToBuilderGraph } from "../builder/app-blueprint";
import { Icon } from "../shell/Icon";
import { lineageStepViews } from "./lineage-step-view";

export function WorkflowDefSection({ handle }: { handle: string }) {
  const navigate = useNavigate();
  const wf = useWorkflow(handle);
  const run = useRunWorkflow();
  const del = useDeleteWorkflow();
  const [historyOpen, setHistoryOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const envelope = (wf.data?.envelope ?? null) as Record<string, unknown> | null;
  const parsed = useMemo(
    () =>
      envelope === null
        ? null
        : appBlueprintToBuilderGraph((envelope.blueprint ?? { seed: 0, steps: [] }) as never),
    [envelope],
  );
  const views = useMemo(
    () =>
      parsed === null || envelope === null
        ? []
        : lineageStepViews(parsed.graph, readModelRoute(envelope)),
    [parsed, envelope],
  );

  if (wf.isLoading) {
    return (
      <section className="screen" data-testid="workflow-def">
        <EmptyState title="Loading workflow…" />
      </section>
    );
  }
  if (wf.isError) {
    const err = toUiError(wf.error);
    return (
      <section className="screen" data-testid="workflow-def">
        {err.kind === "not-wired" ? (
          <EmptyState
            title="Workflows need a newer server"
            detail="This gateway predates durable workflows — upgrade the serve to store and run them."
          />
        ) : (
          <ErrorNotice error={err} onRetry={() => void wf.refetch()} />
        )}
      </section>
    );
  }
  const data = wf.data;
  if (data === null || data === undefined || envelope === null) {
    return (
      <section className="screen" data-testid="workflow-def">
        <EmptyState
          title="Workflow not found"
          detail={`Nothing is stored at ${handle} — it may have been deleted.`}
        />
      </section>
    );
  }

  const name = typeof envelope.name === "string" && envelope.name !== "" ? envelope.name : handle;
  const description = typeof envelope.description === "string" ? envelope.description : "";
  const draft = data.lifecycle === "draft";
  const stepCount = data.stepCount > 0 ? data.stepCount : views.length;

  function onRun(): void {
    run.mutate(
      { handle },
      {
        onSuccess: (started) =>
          void navigate({
            to: "/workflows/$instanceId",
            params: { instanceId: started.instanceId },
            search: runViewSearch(started),
          }),
      },
    );
  }

  return (
    <section className="screen" data-testid="workflow-def">
      <div className="screen__head">
        <div>
          <h1 data-testid="workflow-def-name">
            {name}
            {draft ? (
              <span
                className="chip chip--draft"
                data-testid="workflow-def-draft"
                title="This workflow is a draft — finish it to run and schedule it"
              >
                Draft
              </span>
            ) : null}
          </h1>
          {description !== "" ? (
            <p className="muted" data-testid="workflow-def-description">
              {description}
            </p>
          ) : null}
          <p className="muted" data-testid="workflow-def-meta">
            {stepCount} step{stepCount === 1 ? "" : "s"} ·{" "}
            <code className="mono" title="The workflow's catalog handle">
              {handle}
            </code>
          </p>
        </div>
        <div className="screen__head-actions">
          {draft ? (
            <button
              type="button"
              className="btn-primary"
              data-testid="workflow-def-finish"
              title="A draft can't run — finish it in the editor first"
              onClick={() => void navigate({ to: "/workflows/create", search: { handle } })}
            >
              Finish draft
            </button>
          ) : (
            <button
              type="button"
              className="iconbtn"
              data-testid="workflow-def-run"
              title="Run this workflow (server-built warrants)"
              aria-label="Run"
              disabled={run.isPending}
              onClick={onRun}
            >
              <Icon name="play" size={18} />
            </button>
          )}
          <button
            type="button"
            className="iconbtn"
            data-testid="workflow-def-history"
            title="Definition history — every saved version, restorable in place"
            aria-label="Definition history"
            onClick={() => setHistoryOpen(true)}
          >
            <Icon name="history" size={18} />
          </button>
          <button
            type="button"
            className="iconbtn"
            data-testid="workflow-def-edit"
            title="Edit this workflow in the builder"
            aria-label="Edit"
            onClick={() => void navigate({ to: "/workflows/create", search: { handle } })}
          >
            <Icon name="settings" size={18} />
          </button>
          <button
            type="button"
            className="iconbtn"
            data-testid="workflow-def-delete"
            title="Delete this workflow (asks for confirmation — it also releases its triggers)"
            aria-label="Delete"
            onClick={() => {
              del.reset();
              setDeleting(true);
            }}
          >
            <Icon name="stop" size={18} />
          </button>
        </div>
      </div>

      {run.isError ? (
        <ErrorNotice error={toUiError(run.error)} onRetry={() => run.reset()} />
      ) : null}

      <h2>Steps</h2>
      {views.length === 0 ? (
        <EmptyState
          title="No steps"
          detail="This workflow's blueprint carries no steps — edit it to add some."
        />
      ) : (
        <ul className="app-history__list" data-testid="workflow-def-steps">
          {views.map((v) => (
            <li
              key={v.id}
              className="app-history__row"
              data-testid={`workflow-def-step-${v.ordinal}`}
            >
              <div className="app-history__meta">
                <span className="app-history__cause" title={`A ${v.kind} step`}>
                  {v.kind}
                </span>
                <span title={v.tooltip !== "" ? v.tooltip : undefined}>
                  {v.ordinal}. {v.title}
                </span>
              </div>
              <div className="app-history__detail">
                {v.model !== null ? (
                  <span className={v.modelInferred ? "muted" : undefined}>{v.model}</span>
                ) : null}
                {v.tools.length > 0 ? (
                  // "requests", never "has": a tool_contract is a WISH the server
                  // intersects against the caller's authority at run.
                  <span>
                    requests {v.tools.map((t) => t.id).join(", ")}
                    {v.toolsOverflow > 0 ? ` +${v.toolsOverflow}` : ""}
                  </span>
                ) : null}
                {v.budget !== null ? <span>{v.budget}</span> : null}
                {v.isEntry ? <span className="muted">entry step</span> : null}
              </div>
            </li>
          ))}
        </ul>
      )}

      {historyOpen ? (
        <HistoryDrawer
          handle={handle}
          title="Definition history"
          blockedMessage={null}
          confirmTitle="Restore this workflow?"
          confirmSubject="the definition"
          causeTitles={{
            baseline: "The earliest definition this workflow has a record of",
            create: "The definition history was created",
            advance: "The definition was saved",
            restore: "An earlier saved definition was restored",
          }}
          causeLabels={{ advance: "Saved" }}
          emptyState={{
            title: "No recorded history yet",
            detail:
              "Every save records this workflow's definition here, and any recorded version can be restored.",
          }}
          testIdPrefix="workflow-history"
          invalidateOnRestore={(endpoint, h) => [
            queryKeys.workflow(endpoint, h),
            queryKeys.workflows(endpoint),
          ]}
          onClose={() => setHistoryOpen(false)}
        />
      ) : null}

      {deleting ? (
        <DeleteWorkflowDialog
          handle={handle}
          pending={del.isPending}
          error={del.isError ? toUiError(del.error).message : null}
          onConfirm={() =>
            del.mutate({ handle }, { onSuccess: () => void navigate({ to: "/workflows" }) })
          }
          onClose={() => setDeleting(false)}
        />
      ) : null}
    </section>
  );
}

/**
 * Confirm a Workflow delete (the DeleteAppDialog recipe: danger copy, the SAFE
 * button focused). The copy names what survives as plainly as what goes: the
 * definition HISTORY and the content-addressed blobs stay, so delete + restore
 * recreates without losing state.
 */
function DeleteWorkflowDialog({
  handle,
  pending,
  error,
  onConfirm,
  onClose,
}: {
  handle: string;
  pending: boolean;
  error: string | null;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    // Focus CANCEL, not the destructive action — a stray Enter must not delete.
    cancelRef.current?.focus();
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Cancel delete"
        onClick={onClose}
      />
      <div className="dialog-center dialog-center--overlay">
        <m.div
          className="dialog-card dialog-card--danger"
          data-testid="workflow-delete-dialog"
          // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; modal semantics via role+aria-label (the DeleteAppDialog precedent)
          role="dialog"
          aria-label={`Delete ${handle}`}
          initial={{ y: 12, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          transition={{ type: "spring", stiffness: 420, damping: 34 }}
        >
          <h2 className="dialog-card__title">Delete workflow</h2>
          <p className="muted">
            Delete <code className="mono">{handle}</code>? This also deregisters its triggers and
            releases its lock.
          </p>
          <p className="muted" data-testid="workflow-delete-kept">
            Kept: the recorded definition history and the content-addressed blobs — restoring a
            recorded version recreates the workflow. Past runs are unaffected.
          </p>
          {error ? (
            <p className="field-error" role="alert" data-testid="workflow-delete-error">
              {error}
            </p>
          ) : null}
          <div className="dialog-card__actions">
            <button ref={cancelRef} type="button" className="btn-ghost" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className="btn-primary"
              data-testid="workflow-delete-submit"
              disabled={pending}
              onClick={onConfirm}
            >
              {pending ? "Deleting…" : "Delete workflow"}
            </button>
          </div>
        </m.div>
      </div>
    </>,
    document.body,
  );
}
