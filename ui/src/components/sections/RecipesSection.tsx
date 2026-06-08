import { useNavigate } from "@tanstack/react-router";
import { type FormEvent, useState } from "react";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useRecipeForm, useRecipes } from "../../kx/use-recipes";
import { useRuns } from "../../kx/use-runs";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { RecipeForm } from "../recipes/RecipeForm";

const FALLBACK_HANDLE = "kx/recipes/echo";
const FALLBACK_ARGS = '{\n  "topic": "hello"\n}';

/**
 * The recipe catalog + submit. When the gateway wires the recipe catalog (UI-2),
 * we list the invocable handles and render each recipe's GENERATED free-param form
 * (`GetRecipeForm`). When it does not (an older gateway → UNIMPLEMENTED), we fall
 * back to the manual handle + JSON-args form. Either way, submitting records the
 * run in the session history and routes to the live run-detail DAG.
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
      <h1>Recipes</h1>
      <p className="muted">
        Pick a recipe, fill its inputs, and run it — watch the run execute as a live DAG.
      </p>

      {recipes.isLoading ? <EmptyState title="Loading recipes…" /> : null}

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

/** The catalog-driven path: a recipe picker + the selected recipe's generated form. */
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
        title="No recipes provisioned"
        detail="This gateway exposes the recipe catalog but provisions no recipes."
      />
    );
  }

  return (
    <div data-testid="recipe-catalog">
      <div className="recipe-picker" role="radiogroup" aria-label="Recipe">
        {handles.map((h) => (
          <button
            key={h}
            type="button"
            data-testid={`recipe-pick-${h}`}
            className={`recipe-chip${h === handle ? " recipe-chip--active" : ""}`}
            aria-pressed={h === handle}
            onClick={() => setSelected(h)}
          >
            {h}
          </button>
        ))}
      </div>

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
          This gateway does not expose the recipe catalog — enter a handle + JSON args directly.
        </p>
      ) : null}
      <form className="invoke-form" onSubmit={submit}>
        <label htmlFor="handle">Recipe handle</label>
        <input
          id="handle"
          value={handle}
          onChange={(e) => setHandle(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
        <label htmlFor="args">Args (JSON object)</label>
        <textarea
          id="args"
          value={argsText}
          onChange={(e) => setArgsText(e.target.value)}
          rows={5}
          spellCheck={false}
        />
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
