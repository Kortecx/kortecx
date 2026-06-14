import { m } from "framer-motion";
import { useEffect } from "react";
import { toUiError } from "../../kx/errors";
import { useRecipeForm } from "../../kx/use-recipes";
import { humanizeHandle } from "../../lib/humanize-handle";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { RecipeForm } from "../recipes/RecipeForm";

/**
 * The blueprint run-form in a slide-over drawer (PR-4.1b): clicking a Blueprint
 * card (or its "Run" menu item) opens this; the clone-lite landing
 * (`/recipes?handle=&args=`) auto-opens it PREFILLED. Reuses the `.node-drawer`
 * skeleton + Escape convention (BlueprintViewer / NodeDetailDrawer). The form
 * itself (`RecipeForm`, testid `recipe-form` + `data-recipe`) is unchanged —
 * the gateway re-validates every arg server-side (SN-8).
 */
export function BlueprintFormDrawer({
  handle,
  prefill,
  pending,
  onRun,
  onClose,
}: {
  handle: string;
  /** Prefill values (the clone-lite landing: a run's prior args). */
  prefill?: Record<string, unknown>;
  pending: boolean;
  onRun: (handle: string, args: Record<string, unknown>) => void;
  onClose: () => void;
}) {
  const form = useRecipeForm(handle);

  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close blueprint form"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
        data-testid="blueprint-form-drawer"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion animations; non-modal side-panel semantics declared via role+aria-label (the NodeDetailDrawer precedent)
        role="dialog"
        aria-label={`Run blueprint ${handle}`}
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <strong>{humanizeHandle(handle)}</strong>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        <code className="mono muted" title={handle}>
          {handle}
        </code>
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
      </m.aside>
    </>
  );
}
