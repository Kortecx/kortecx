import { Link, useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRecipeSummaries, useRecipes } from "../../kx/use-recipes";
import { useRuns } from "../../kx/use-runs";
import { type WorkflowSummary, useRunWorkflow, useWorkflows } from "../../kx/use-workflows";
import { BLUEPRINT_NAMES_CHANGED_EVENT, loadBlueprintNames } from "../../lib/blueprint-names";
import { humanizeHandle } from "../../lib/humanize-handle";
import { runViewSearch } from "../../lib/run-anchor";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { Icon } from "../shell/Icon";
import { BlueprintFormDrawer } from "./BlueprintFormDrawer";
import { WorkflowCard } from "./WorkflowCard";

/** The display headline a workflow card renders. */
interface WorkflowDisplay {
  readonly headline: string;
  readonly customName: string | null;
}

/**
 * The Workflows CATALOG tab — YOUR durable workflows first (`ListWorkflows`,
 * The card face: name · description · steps · draft badge; the card opens the definition
 * page, Run fires `RunWorkflow` server-side), then the ready-made blueprints the
 * gateway ships with as a demoted "Built-in" group (`ListRecipes` — server-fixed;
 * its run flow is the shipped `Invoke` form drawer, unchanged).
 */
export function WorkflowsCatalog() {
  const navigate = useNavigate();
  const { endpoint } = useConnection();
  const { add } = useRuns();
  const invoke = useInvoke();
  const recipes = useRecipes();
  const summaries = useRecipeSummaries();
  const mine = useWorkflows();
  const runWorkflow = useRunWorkflow();
  const [names, setNames] = useState<Record<string, string>>(() => loadBlueprintNames(endpoint));
  const [openForm, setOpenForm] = useState<string | null>(null);

  // Stay fresh across client-local rename events + endpoint switches.
  useEffect(() => {
    setNames(loadBlueprintNames(endpoint));
    function onNamesChanged(): void {
      setNames(loadBlueprintNames(endpoint));
    }
    window.addEventListener(BLUEPRINT_NAMES_CHANGED_EVENT, onNamesChanged);
    return () => window.removeEventListener(BLUEPRINT_NAMES_CHANGED_EVENT, onNamesChanged);
  }, [endpoint]);

  function start(handle: string, args: Record<string, unknown>): void {
    invoke.mutate(
      { handle, args },
      {
        onSuccess: (started) => {
          add({
            instanceId: started.instanceId,
            terminalMoteId: started.terminalMoteId,
            // Persist the chain key too, so reopening this run from history stays scoped.
            reactChainSalt: started.reactChainSalt,
            recipeFingerprint: started.recipeFingerprint,
            handle,
            startedAt: Date.now(),
            args: JSON.stringify(args),
          });
          navigate({
            to: "/workflows/$instanceId",
            params: { instanceId: started.instanceId },
            search: runViewSearch(started),
          });
        },
      },
    );
  }

  /** Display name precedence: local rename > humanized handle. */
  function nameFor(handle: string): WorkflowDisplay {
    const local = names[handle];
    const customName = local && local.trim() !== "" ? local : null;
    return { headline: customName ?? humanizeHandle(handle), customName };
  }

  /** Run a stored workflow SERVER-side and land on the scoped run view. */
  function startWorkflow(handle: string): void {
    runWorkflow.mutate(
      { handle },
      {
        onSuccess: (started) => {
          navigate({
            to: "/workflows/$instanceId",
            params: { instanceId: started.instanceId },
            search: runViewSearch(started),
          });
        },
      },
    );
  }

  const catalog = recipes.data;
  const catalogUnavailable = recipes.isError && toUiError(recipes.error).kind === "not-wired";
  const invokeError = invoke.error ? toUiError(invoke.error) : null;
  const runWorkflowError = runWorkflow.error ? toUiError(runWorkflow.error) : null;

  return (
    <div data-testid="workflows-tab">
      {/* YOUR workflows — the durable `kortecx.workflow/v1` catalog. On a
          gateway without the store the group degrades away (not-wired), leaving
          exactly the pre-W3 surface. */}
      {mine.notWired ? null : (
        <>
          <h2>Your workflows</h2>
          {mine.isLoading ? (
            <EmptyState title="Loading your workflows…" />
          ) : mine.workflows.length === 0 ? (
            <EmptyState
              title="No saved workflows yet"
              detail="A workflow you save is durable — runnable and schedulable by handle, with every saved version restorable. Build one to get started."
              action={
                <Link
                  to="/workflows/create"
                  className="btnlink"
                  data-testid="workflows-mine-empty-create"
                >
                  New workflow →
                </Link>
              }
            />
          ) : (
            <m.div
              className="card-grid"
              data-testid="workflows-mine"
              variants={stagger()}
              initial="hidden"
              animate="show"
            >
              {mine.workflows.map((w) => (
                <StoredWorkflowCard
                  key={w.handle}
                  workflow={w}
                  runPending={runWorkflow.isPending}
                  onOpen={(handle) =>
                    void navigate({ to: "/workflows/def/$handle", params: { handle } })
                  }
                  onRun={startWorkflow}
                />
              ))}
            </m.div>
          )}
          {runWorkflowError ? (
            <ErrorNotice error={runWorkflowError} onRetry={() => runWorkflow.reset()} />
          ) : null}
          <h2>Built-in</h2>
        </>
      )}

      {recipes.isLoading ? <EmptyState title="Loading workflows…" /> : null}

      {catalog ? (
        catalog.length === 0 ? (
          // This list is `ListRecipes` — the `kx/recipes/*` handles the GATEWAY
          // provisions. It is server-fixed: nothing the user builds is ever published
          // into it. What you build durably is a WORKFLOW — authored at
          // /workflows/create, listed above under "Your workflows".
          <EmptyState
            title="This gateway publishes no built-in workflows"
            detail="The built-in list is the ready-made workflows a gateway ships with, fixed by the server — what you build is never added to it. Your own workflows live above: author one in the builder and Save; it is then runnable and schedulable by handle."
            action={
              <Link
                to="/workflows/create"
                className="btnlink"
                data-testid="workflows-empty-create-link"
              >
                New workflow →
              </Link>
            }
          />
        ) : (
          <m.div
            className="card-grid"
            data-testid="workflows-catalog"
            variants={stagger()}
            initial="hidden"
            animate="show"
          >
            {catalog.map((h) => {
              const d = nameFor(h);
              return (
                <WorkflowCard
                  key={h}
                  handle={h}
                  headline={d.headline}
                  customName={d.customName}
                  summary={summaries.data?.[h]}
                  onRun={setOpenForm}
                />
              );
            })}
          </m.div>
        )
      ) : null}

      {catalogUnavailable ? (
        <EmptyState
          title="Workflow catalog not available"
          detail="This gateway does not expose the workflow catalog (an older build)."
        />
      ) : null}

      {invokeError ? <ErrorNotice error={invokeError} onRetry={() => invoke.reset()} /> : null}

      {openForm ? (
        <BlueprintFormDrawer
          handle={openForm}
          pending={invoke.isPending}
          onRun={start}
          onClose={() => setOpenForm(null)}
        />
      ) : null}

      <p className="muted" data-testid="workflows-apps-hint">
        Looking for a saved App? Run, create, and manage Apps in the{" "}
        <Link to="/apps" data-testid="workflows-apps-link">
          Apps
        </Link>{" "}
        section — each App runs from its typed input drawer.
      </p>
    </div>
  );
}

/**
 * One STORED workflow — name · description · step count · draft badge.
 * The title opens the definition page (`/workflows/def/$handle`); Run fires
 * `RunWorkflow` server-side. A DRAFT swaps Run for "Finish draft": the server
 * refuses to schedule a draft, so offering Run here would promise what the
 * definition page (honestly) refuses.
 */
function StoredWorkflowCard({
  workflow,
  runPending,
  onOpen,
  onRun,
}: {
  workflow: WorkflowSummary;
  runPending: boolean;
  onOpen: (handle: string) => void;
  onRun: (handle: string) => void;
}) {
  const draft = workflow.lifecycle === "draft";
  return (
    <m.article
      className="glow-card glow-card--hover card-grid__card"
      data-testid={`workflow-def-card-${workflow.handle}`}
      variants={fadeUp}
      {...hoverLift}
    >
      <div className="card-grid__head">
        <button
          type="button"
          className="card-grid__title card-grid__title-btn"
          data-testid={`workflow-def-open-${workflow.handle}`}
          title={`${workflow.name} — view details`}
          onClick={() => onOpen(workflow.handle)}
        >
          {workflow.name !== "" ? workflow.name : workflow.handle}
        </button>
        {draft ? (
          <span
            className="chip chip--draft"
            data-testid={`workflow-draft-${workflow.handle}`}
            title="This workflow is a draft — finish it to run and schedule it"
          >
            Draft
          </span>
        ) : null}
        <div className="card-grid__head-actions">
          {draft ? (
            <Link
              to="/workflows/create"
              search={{ handle: workflow.handle }}
              className="btn-ghost"
              data-testid={`workflow-def-finish-${workflow.handle}`}
              title="Finish this draft in the editor"
            >
              Finish draft
            </Link>
          ) : (
            <button
              type="button"
              className="iconbtn"
              data-testid={`workflow-def-run-${workflow.handle}`}
              title="Run this workflow"
              aria-label="Run"
              disabled={runPending}
              onClick={() => onRun(workflow.handle)}
            >
              <Icon name="play" size={16} />
            </button>
          )}
        </div>
      </div>
      {workflow.description !== "" ? (
        <p className="card-grid__sub">{workflow.description}</p>
      ) : null}
      <p className="muted">
        {workflow.stepCount} step{workflow.stepCount === 1 ? "" : "s"}
      </p>
    </m.article>
  );
}
