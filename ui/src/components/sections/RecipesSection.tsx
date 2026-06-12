import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { type FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { rowEntrance } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRecipeForm, useRecipes } from "../../kx/use-recipes";
import { useRuns } from "../../kx/use-runs";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { CodeViewer } from "../editor/CodeViewer";
import { JsonEditor } from "../editor/JsonEditor";
import { RecipeForm } from "../recipes/RecipeForm";

const FALLBACK_HANDLE = "kx/recipes/echo";
const FALLBACK_ARGS = '{\n  "topic": "hello"\n}';

/**
 * The Blueprint catalog + submit (display name for the frozen `recipe` wire).
 * PR-2.1: the catalog is a TABLE — one blueprint per row with per-row controls
 * (Run → the form below; View → the contract in a read-only Monaco popup) —
 * and the route accepts `?handle=&args=` to land with a prior run's inputs
 * PREFILLED (the clone-lite flow from Workflows). Sharing across parties is a
 * cloud capability (D129) — no fake control here.
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
  const { add } = useRuns();
  const invoke = useInvoke();
  const recipes = useRecipes();

  function start(handle: string, args: Record<string, unknown>): void {
    invoke.mutate(
      { handle, args },
      {
        onSuccess: ({ instanceId, terminalMoteId, recipeFingerprint }) => {
          add({
            instanceId,
            terminalMoteId,
            recipeFingerprint,
            handle,
            startedAt: Date.now(),
            // PR-2.1: keep the args so the Workflows row can Run-again/Clone.
            args: JSON.stringify(args),
          });
          navigate({
            to: "/workflows/$instanceId",
            params: { instanceId },
            search: { terminal: terminalMoteId },
          });
        },
      },
    );
  }

  const invokeError = invoke.error ? toUiError(invoke.error) : null;
  const catalogUnavailable = recipes.isError && toUiError(recipes.error).kind === "not-wired";

  return (
    <section className="screen" data-testid="recipes-section">
      <h1>Blueprints</h1>
      <p className="muted">
        Pick a blueprint, fill its inputs, and run it — watch the run execute as a live DAG.
      </p>

      {recipes.isLoading ? <EmptyState title="Loading blueprints…" /> : null}

      {recipes.data ? (
        <RecipeTable
          handles={recipes.data}
          pending={invoke.isPending}
          onRun={start}
          initialHandle={initialHandle}
          initialArgs={initialArgs}
        />
      ) : null}

      {catalogUnavailable || (recipes.isError && !recipes.data) ? (
        <ManualInvokeForm pending={invoke.isPending} onRun={start} degraded={catalogUnavailable} />
      ) : null}

      {invokeError ? <ErrorNotice error={invokeError} onRetry={() => invoke.reset()} /> : null}
    </section>
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

/** The catalog table: one blueprint per row + per-row Run/View controls. */
function RecipeTable({
  handles,
  pending,
  onRun,
  initialHandle,
  initialArgs,
}: {
  handles: string[];
  pending: boolean;
  onRun: (handle: string, args: Record<string, unknown>) => void;
  initialHandle?: string;
  initialArgs?: string;
}) {
  const [selected, setSelected] = useState(() =>
    initialHandle && handles.includes(initialHandle)
      ? initialHandle
      : (handles[0] ?? FALLBACK_HANDLE),
  );
  const handle = handles.includes(selected) ? selected : (handles[0] ?? FALLBACK_HANDLE);
  const form = useRecipeForm(handle);
  const [viewing, setViewing] = useState<string | null>(null);
  const formRef = useRef<HTMLDivElement | null>(null);
  // Only the clone-lite landing's TARGET blueprint gets the prefill.
  const prefill = useMemo(
    () => (handle === initialHandle ? parsePrefill(initialArgs) : undefined),
    [handle, initialHandle, initialArgs],
  );

  if (handles.length === 0) {
    return (
      <EmptyState
        title="No blueprints provisioned"
        detail="This gateway exposes the blueprint catalog but provisions no blueprints."
      />
    );
  }

  return (
    <div data-testid="recipe-catalog">
      <table className="recipe-table" data-testid="recipe-table">
        <thead>
          <tr>
            <th scope="col">Blueprint</th>
            <th scope="col" className="recipe-table__actions">
              Actions
            </th>
          </tr>
        </thead>
        <tbody>
          {handles.map((h, i) => (
            <m.tr
              key={h}
              className={h === handle ? "recipe-row recipe-row--active" : "recipe-row"}
              data-testid={`recipe-row-${h}`}
              {...rowEntrance(i)}
            >
              <td>
                <button
                  type="button"
                  data-testid={`recipe-pick-${h}`}
                  className={`recipe-chip${h === handle ? " recipe-chip--active" : ""}`}
                  aria-pressed={h === handle}
                  onClick={() => setSelected(h)}
                >
                  {h}
                </button>
              </td>
              <td className="recipe-table__actions">
                <button
                  type="button"
                  className="linkbtn"
                  data-testid={`recipe-run-${h}`}
                  title="Open this blueprint's input form below"
                  onClick={() => {
                    setSelected(h);
                    formRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
                  }}
                >
                  Run
                </button>
                <button
                  type="button"
                  className="linkbtn"
                  data-testid={`recipe-view-${h}`}
                  title="View the blueprint's contract (inputs + types)"
                  onClick={() => setViewing(h)}
                >
                  View
                </button>
              </td>
            </m.tr>
          ))}
        </tbody>
      </table>

      <div ref={formRef}>
        {form.isLoading ? <EmptyState title="Loading form…" /> : null}
        {form.error ? (
          <ErrorNotice error={toUiError(form.error)} onRetry={() => void form.refetch()} />
        ) : null}
        {form.data ? (
          <RecipeForm
            // Re-key per handle+prefill so a clone-landing remount prefills.
            key={`${handle}:${prefill ? "prefilled" : "blank"}`}
            form={form.data}
            pending={pending}
            onSubmit={(args) => onRun(handle, args)}
            initial={prefill}
          />
        ) : null}
      </div>

      {viewing ? <BlueprintViewer handle={viewing} onClose={() => setViewing(null)} /> : null}
    </div>
  );
}

/**
 * The blueprint-contract popup (PR-2.1): the handle + its full free-param
 * contract rendered as JSON in the read-only Monaco viewer (D141.2). Pure
 * display — the contract is exactly what `GetRecipeForm` declares.
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
    ? JSON.stringify(
        {
          handle: form.data.handle,
          inputs: form.data.fields.map((f) => ({
            name: f.name,
            type: f.type,
            required: f.required,
            ...(f.maxLen ? { max_len: f.maxLen } : {}),
            ...(f.allowed.length > 0 ? { allowed: f.allowed } : {}),
          })),
        },
        null,
        2,
      )
    : null;

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close blueprint view"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
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
    </>
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
