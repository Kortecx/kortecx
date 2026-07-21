import { Link, useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useState } from "react";
import { stagger } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRecipeSummaries, useRecipes } from "../../kx/use-recipes";
import { useRuns } from "../../kx/use-runs";
import { BLUEPRINT_NAMES_CHANGED_EVENT, loadBlueprintNames } from "../../lib/blueprint-names";
import { humanizeHandle } from "../../lib/humanize-handle";
import { runViewSearch } from "../../lib/run-anchor";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { BlueprintFormDrawer } from "./BlueprintFormDrawer";
import { WorkflowCard } from "./WorkflowCard";

/** The display headline a workflow card renders. */
interface WorkflowDisplay {
  readonly headline: string;
  readonly customName: string | null;
}

/**
 * The Workflows CATALOG tab — the runnable workflows the gateway provisions, as a
 * clean card grid (name · description · Run/Schedule/Share + kebab). Reuses the
 * shipped run flow: clicking Run opens the input form (`BlueprintFormDrawer`),
 * submit `Invoke`s and navigates to the live run at `/workflows/$instanceId`.
 * High-level only — no raw handles, no lock (workflows aren't lockable).
 */
export function WorkflowsCatalog() {
  const navigate = useNavigate();
  const { endpoint } = useConnection();
  const { add } = useRuns();
  const invoke = useInvoke();
  const recipes = useRecipes();
  const summaries = useRecipeSummaries();
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

  const catalog = recipes.data;
  const catalogUnavailable = recipes.isError && toUiError(recipes.error).kind === "not-wired";
  const invokeError = invoke.error ? toUiError(invoke.error) : null;

  return (
    <div data-testid="workflows-tab">
      {recipes.isLoading ? <EmptyState title="Loading workflows…" /> : null}

      {catalog ? (
        catalog.length === 0 ? (
          // This list is `ListRecipes` — the `kx/recipes/*` handles the GATEWAY
          // provisions. It is server-fixed: nothing the user builds is ever published
          // into it (the builder's submit is an ad-hoc `SubmitWorkflow` DAG; its durable
          // form is "Save as App"). The old copy promised the opposite, so a user who
          // authored a workflow came back here and found an empty list with no
          // explanation.
          <EmptyState
            title="This gateway publishes no workflows"
            detail="The catalog lists the ready-made workflows a gateway ships with. It is fixed by the server — what you build here is never added to it. Save a workflow you author in the builder with “Save as App”; it then lives in Apps, where you can run and schedule it."
            action={
              <Link to="/apps" className="btnlink" data-testid="workflows-empty-apps-link">
                Go to Apps →
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
