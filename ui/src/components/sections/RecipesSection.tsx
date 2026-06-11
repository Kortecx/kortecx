import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { type CSSProperties, type FormEvent, useState } from "react";
import { fadeUp, hoverLiftLarge, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRecipeForm, useRecipes } from "../../kx/use-recipes";
import { useRuns } from "../../kx/use-runs";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { JsonEditor } from "../editor/JsonEditor";
import { RecipeForm } from "../recipes/RecipeForm";

const FALLBACK_HANDLE = "kx/recipes/echo";
const FALLBACK_ARGS = '{\n  "topic": "hello"\n}';

/** The tile's top accent stripe, keyed by the blueprint category in the handle. */
function stripeColorFor(handle: string): string {
  if (handle.includes("echo")) return "var(--primary)";
  if (handle.includes("plan")) return "var(--info)";
  if (handle.includes("react")) return "var(--violet)";
  if (handle.includes("exec")) return "var(--teal)";
  return "var(--primary)";
}

/**
 * The Blueprint catalog + submit (display name for the frozen `recipe` wire — the
 * RPCs stay `ListRecipes`/`GetRecipeForm`, handles stay `kx/recipes/*`). When the
 * gateway wires the catalog (UI-2), we list the invocable handles and render each
 * blueprint's GENERATED free-param form. When it does not (an older gateway →
 * UNIMPLEMENTED), we fall back to the manual handle + JSON-args form. Either way,
 * submitting records the run in the session history and routes to the live DAG.
 */
export function RecipesSection() {
  const navigate = useNavigate();
  const { add } = useRuns();
  const invoke = useInvoke();
  const recipes = useRecipes();

  function start(handle: string, args: Record<string, unknown>): void {
    invoke.mutate(
      { handle, args },
      {
        onSuccess: ({ instanceId, terminalMoteId }) => {
          add({
            instanceId,
            terminalMoteId,
            recipeFingerprint: null,
            handle,
            startedAt: Date.now(),
          });
          navigate({
            to: "/runs/$instanceId",
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
        <RecipeCatalog handles={recipes.data} pending={invoke.isPending} onRun={start} />
      ) : null}

      {catalogUnavailable || (recipes.isError && !recipes.data) ? (
        <ManualInvokeForm pending={invoke.isPending} onRun={start} degraded={catalogUnavailable} />
      ) : null}

      {invokeError ? <ErrorNotice error={invokeError} onRetry={() => invoke.reset()} /> : null}
    </section>
  );
}

/** The catalog-driven path: a blueprint picker + the selected blueprint's generated form. */
function RecipeCatalog({
  handles,
  pending,
  onRun,
}: {
  handles: string[];
  pending: boolean;
  onRun: (handle: string, args: Record<string, unknown>) => void;
}) {
  const [selected, setSelected] = useState(() => handles[0] ?? FALLBACK_HANDLE);
  const handle = handles.includes(selected) ? selected : (handles[0] ?? FALLBACK_HANDLE);
  const form = useRecipeForm(handle);

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
      <m.div
        className="recipe-picker"
        role="radiogroup"
        aria-label="Blueprint"
        variants={stagger()}
        initial="hidden"
        animate="show"
      >
        {handles.map((h) => (
          <m.div
            key={h}
            className={`glow-card glow-card--stripe card-hover recipe-tile${
              h === handle ? " recipe-tile--active" : ""
            }`}
            style={{ "--stripe": stripeColorFor(h) } as CSSProperties}
            variants={fadeUp}
            {...hoverLiftLarge}
          >
            <button
              type="button"
              data-testid={`recipe-pick-${h}`}
              className={`recipe-chip${h === handle ? " recipe-chip--active" : ""}`}
              aria-pressed={h === handle}
              onClick={() => setSelected(h)}
            >
              {h}
            </button>
          </m.div>
        ))}
      </m.div>

      {form.isLoading ? <EmptyState title="Loading form…" /> : null}
      {form.error ? (
        <ErrorNotice error={toUiError(form.error)} onRetry={() => void form.refetch()} />
      ) : null}
      {form.data ? (
        <RecipeForm form={form.data} pending={pending} onSubmit={(args) => onRun(handle, args)} />
      ) : null}
    </div>
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
