import type { RecipeInfo } from "@kortecx/sdk/web";
import { Link, useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { type FormEvent, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { stagger } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRecipeSearch } from "../../kx/use-recipe-search";
import { useRecipeForm, useRecipeSummaries, useRecipes } from "../../kx/use-recipes";
import { useRuns } from "../../kx/use-runs";
import { BLUEPRINT_NAMES_CHANGED_EVENT, loadBlueprintNames } from "../../lib/blueprint-names";
import { blueprintInputs } from "../../lib/export-blueprint";
import { humanizeHandle } from "../../lib/humanize-handle";
import { runViewSearch } from "../../lib/run-anchor";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { CodeViewer } from "../editor/CodeViewer";
import { JsonEditor } from "../editor/JsonEditor";
import { BlueprintCard } from "./BlueprintCard";
import { BlueprintFormDrawer } from "./BlueprintFormDrawer";

const FALLBACK_HANDLE = "kx/recipes/echo";
const FALLBACK_ARGS = '{\n  "topic": "hello"\n}';

/** The display shape a Blueprint card renders. */
interface BlueprintDisplay {
  readonly headline: string;
  readonly customName: string | null;
}

/**
 * The Blueprint catalog + submit (display name for the frozen `recipe` wire).
 * PR-4.1b: the catalog is a CARD GRID — one blueprint per card with a clean
 * display name, a description subtitle + advisory tag/version chips, and a
 * per-card action menu (Run · Open-in-new-tab · View contract · Edit in builder
 * · Rename · Export · Share[Cloud] · Schedule[Cloud]). Clicking a card opens
 * the run form in a slide-over drawer; `?handle=&args=` auto-opens it PREFILLED
 * (the clone-lite flow from Workflows). Sharing across parties + scheduling are
 * cloud capabilities (D129) — honest-disabled chips, never fake controls.
 */
export function RecipesSection({
  initialHandle,
  initialArgs,
}: {
  /** Preselect this blueprint (the `?handle=` search param). */
  initialHandle?: string;
  /** Prefill the form with these args (JSON text — the `?args=` search param). */
  initialArgs?: string;
}) {
  const navigate = useNavigate();
  const { endpoint } = useConnection();
  const { add } = useRuns();
  const invoke = useInvoke();
  const recipes = useRecipes();
  const summaries = useRecipeSummaries();
  const [names, setNames] = useState<Record<string, string>>(() => loadBlueprintNames(endpoint));
  const [openForm, setOpenForm] = useState<{
    handle: string;
    prefill?: Record<string, unknown>;
  } | null>(null);
  const [viewing, setViewing] = useState<string | null>(null);

  // Stay fresh across blueprint-rename events + endpoint switches.
  useEffect(() => {
    setNames(loadBlueprintNames(endpoint));
    function onNamesChanged(): void {
      setNames(loadBlueprintNames(endpoint));
    }
    window.addEventListener(BLUEPRINT_NAMES_CHANGED_EVENT, onNamesChanged);
    return () => window.removeEventListener(BLUEPRINT_NAMES_CHANGED_EVENT, onNamesChanged);
  }, [endpoint]);

  // Clone-lite landing: when `?handle=` targets a provisioned blueprint, open
  // its run form PREFILLED (fires once the catalog has loaded).
  const catalog = recipes.data;
  useEffect(() => {
    if (initialHandle && catalog && catalog.includes(initialHandle)) {
      setOpenForm({ handle: initialHandle, prefill: parsePrefill(initialArgs) });
    }
  }, [initialHandle, initialArgs, catalog]);

  function start(handle: string, args: Record<string, unknown>): void {
    invoke.mutate(
      { handle, args },
      {
        onSuccess: (started) => {
          add({
            instanceId: started.instanceId,
            terminalMoteId: started.terminalMoteId,
            // Persist the chain key too — reopening this run from history must stay
            // scoped to it, and only the submit response knows the salt.
            reactChainSalt: started.reactChainSalt,
            recipeFingerprint: started.recipeFingerprint,
            handle,
            startedAt: Date.now(),
            // Keep the args so the Workflows card can Run-again/Clone.
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
  function nameFor(handle: string): BlueprintDisplay {
    const local = names[handle];
    const customName = local && local.trim() !== "" ? local : null;
    return { headline: customName ?? humanizeHandle(handle), customName };
  }

  const invokeError = invoke.error ? toUiError(invoke.error) : null;
  const catalogUnavailable = recipes.isError && toUiError(recipes.error).kind === "not-wired";

  return (
    <section className="screen" data-testid="recipes-section">
      <div className="section-head">
        <div>
          <h1>Blueprints</h1>
          <p className="muted">
            Pick a blueprint, fill its inputs, and run it — watch the run execute as a live DAG.
          </p>
        </div>
        <Link to="/blueprints/new" className="builder-newlink" data-testid="new-blueprint">
          + New blueprint
        </Link>
      </div>

      <BlueprintDiscovery />

      {recipes.isLoading ? <EmptyState title="Loading blueprints…" /> : null}

      {catalog ? (
        <BlueprintCatalog
          handles={catalog}
          summaries={summaries.data ?? {}}
          nameFor={nameFor}
          onRun={(handle) => setOpenForm({ handle })}
          onView={setViewing}
        />
      ) : null}

      {catalogUnavailable || (recipes.isError && !recipes.data) ? (
        <ManualInvokeForm pending={invoke.isPending} onRun={start} degraded={catalogUnavailable} />
      ) : null}

      {invokeError ? <ErrorNotice error={invokeError} onRetry={() => invoke.reset()} /> : null}

      {openForm ? (
        <BlueprintFormDrawer
          handle={openForm.handle}
          prefill={openForm.prefill}
          pending={invoke.isPending}
          onRun={start}
          onClose={() => setOpenForm(null)}
        />
      ) : null}

      {viewing ? <BlueprintViewer handle={viewing} onClose={() => setViewing(null)} /> : null}
    </section>
  );
}

/** The catalog as a card grid — one {@link BlueprintCard} per provisioned handle. */
function BlueprintCatalog({
  handles,
  summaries,
  nameFor,
  onRun,
  onView,
}: {
  handles: string[];
  summaries: Record<string, RecipeInfo>;
  nameFor: (handle: string) => BlueprintDisplay;
  onRun: (handle: string) => void;
  onView: (handle: string) => void;
}) {
  if (handles.length === 0) {
    return (
      <EmptyState
        title="No blueprints provisioned"
        detail="This gateway exposes the blueprint catalog but provisions no blueprints."
      />
    );
  }
  return (
    <m.div
      className="card-grid"
      data-testid="recipe-catalog"
      variants={stagger()}
      initial="hidden"
      animate="show"
    >
      {handles.map((h) => {
        const d = nameFor(h);
        return (
          <BlueprintCard
            key={h}
            handle={h}
            headline={d.headline}
            customName={d.customName}
            summary={summaries[h]}
            onRun={onRun}
            onView={onView}
          />
        );
      })}
    </m.div>
  );
}

/**
 * Advisory recipe discovery (PR-4 Batch D `SearchRecipes`): type an intent, see
 * the gateway's recipes ranked by match (display-only basis points — a hit
 * surfaces a recipe, never invokes it). Selecting a result re-lands the section
 * with that blueprint preselected. Hidden when the gateway has no ranker.
 */
function BlueprintDiscovery() {
  const [intent, setIntent] = useState("");
  const { results, unsupported, loading } = useRecipeSearch(intent);
  if (unsupported) {
    return null;
  }
  return (
    <div className="blueprint-search" data-testid="blueprint-search">
      <input
        className="blueprint-search__input"
        type="search"
        placeholder="Search blueprints by intent (e.g. “agent loop”, “chat”)…"
        value={intent}
        onChange={(e) => setIntent(e.target.value)}
        data-testid="blueprint-search-input"
        aria-label="Search blueprints"
      />
      {loading ? <span className="muted blueprint-search__hint">Searching…</span> : null}
      {results && intent.trim() ? (
        results.length === 0 ? (
          <p className="muted blueprint-search__hint">No matching blueprints.</p>
        ) : (
          <ul className="blueprint-search__results" data-testid="blueprint-search-results">
            {results.map((r) => (
              <li key={r.recipe.handle}>
                <Link
                  to="/recipes"
                  search={{ handle: r.recipe.handle }}
                  className="blueprint-search__hit"
                  data-testid="blueprint-search-hit"
                >
                  <span className="mono blueprint-search__handle">{r.recipe.handle}</span>
                  {r.recipe.description ? (
                    <span className="blueprint-search__desc">{r.recipe.description}</span>
                  ) : null}
                  <span className="blueprint-search__tags">
                    {r.recipe.tags.map((t) => (
                      <span key={t} className="chip chip--tag">
                        {t}
                      </span>
                    ))}
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        )
      ) : null}
    </div>
  );
}

/** Parse the prefill args JSON (fail-closed: bad text prefills nothing). */
function parsePrefill(argsText: string | undefined): Record<string, unknown> | undefined {
  if (!argsText) {
    return undefined;
  }
  try {
    const parsed: unknown = JSON.parse(argsText);
    if (parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
  } catch {
    /* ignore — the form starts empty */
  }
  return undefined;
}

/**
 * The blueprint-contract popup (PR-2.1): the handle + its full free-param
 * contract rendered as JSON in the read-only Monaco viewer (D141.2). Pure
 * display — the contract is exactly what `GetRecipeForm` declares (shaped by the
 * shared `blueprintInputs`, so the viewer + the Export file never drift).
 */
function BlueprintViewer({ handle, onClose }: { handle: string; onClose: () => void }) {
  const form = useRecipeForm(handle);

  // Close on Escape (the drawer convention).
  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const contract = form.data
    ? JSON.stringify({ handle: form.data.handle, inputs: blueprintInputs(form.data) }, null, 2)
    : null;

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close blueprint view"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay"
        data-testid="blueprint-viewer"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion animations; non-modal side panel semantics declared via role+aria-label (the NodeDetailDrawer precedent)
        role="dialog"
        aria-label={`Blueprint ${handle}`}
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <code className="mono node-drawer__id" title={handle}>
            {handle}
          </code>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        {form.isLoading ? <EmptyState title="Loading contract…" /> : null}
        {form.error ? <ErrorNotice error={toUiError(form.error)} /> : null}
        {contract ? (
          <CodeViewer
            value={contract}
            language="json"
            testId="blueprint-contract"
            ariaLabel={`Blueprint contract ${handle}`}
            height={Math.min(420, Math.max(140, contract.split("\n").length * 19 + 24))}
          />
        ) : null}
      </m.aside>
    </>,
    document.body,
  );
}

/** The fallback path: a raw handle + JSON-args form (older gateways without the catalog). */
function ManualInvokeForm({
  pending,
  onRun,
  degraded,
}: {
  pending: boolean;
  onRun: (handle: string, args: Record<string, unknown>) => void;
  degraded: boolean;
}) {
  const [handle, setHandle] = useState(FALLBACK_HANDLE);
  const [argsText, setArgsText] = useState(FALLBACK_ARGS);
  const [argsError, setArgsError] = useState<string | null>(null);

  function submit(e: FormEvent<HTMLFormElement>): void {
    e.preventDefault();
    let args: Record<string, unknown>;
    try {
      const parsed: unknown = JSON.parse(argsText);
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("args must be a JSON object");
      }
      args = parsed as Record<string, unknown>;
    } catch (err) {
      setArgsError((err as Error).message);
      return;
    }
    setArgsError(null);
    onRun(handle.trim(), args);
  }

  return (
    <div data-testid="manual-invoke">
      {degraded ? (
        <p className="muted">
          This gateway does not expose the blueprint catalog — enter a handle + JSON args directly.
        </p>
      ) : null}
      <form className="invoke-form" onSubmit={submit}>
        <label htmlFor="handle">Blueprint handle</label>
        <input
          id="handle"
          value={handle}
          onChange={(e) => setHandle(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
        <label htmlFor="args">Args (JSON object)</label>
        <JsonEditor id="args" value={argsText} onChange={setArgsText} />
        {argsError ? (
          <p className="field-error" role="alert">
            {argsError}
          </p>
        ) : null}
        <button type="submit" disabled={pending}>
          {pending ? "Submitting…" : "Submit run"}
        </button>
      </form>
    </div>
  );
}
