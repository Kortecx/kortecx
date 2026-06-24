/**
 * POC-5d: the single-App RUN drawer — a slide-over (the `.node-drawer` design
 * language) that reads the App's `input_schema` inputs (via {@link appInputForm} →
 * the existing {@link RecipeForm} renderer), validates them, and triggers a single
 * run (`useRunApp` → `GetApp` → `SubmitWorkflow`, the args folded into the entry
 * model step's prompt server-side-equivalently). On submit it routes to the live run
 * (`/workflows/$instanceId`). An App with no input fields runs directly with one
 * click (no form). SN-8: the server re-resolves every warrant from the caller's
 * grants — args steer, never grant.
 */

import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useMemo } from "react";
import { toUiError } from "../../kx/errors";
import { useApp, useRunApp } from "../../kx/use-apps";
import { appInputForm } from "../../lib/app-input-schema";
import { ErrorNotice } from "../ErrorNotice";
import { RecipeForm } from "../recipes/RecipeForm";

// Mirror the StepConfigDrawer slide-over motion (kept local — not exported).
const slideIn = {
  initial: { x: 24, opacity: 0 },
  animate: { x: 0, opacity: 1 },
  transition: { type: "spring", stiffness: 420, damping: 34 },
} as const;

export function AppRunDrawer({ handle, onClose }: { handle: string; onClose: () => void }) {
  const navigate = useNavigate();
  const runApp = useRunApp();
  // The App's input_schema → the typed run form (react-query dedupes with any
  // already-loaded GetApp — e.g. the IDE that opened this drawer).
  const app = useApp(handle);
  const inputSchema = (app.data?.envelope.input_schema ?? null) as unknown;
  const form = useMemo(() => appInputForm(handle, inputSchema), [handle, inputSchema]);

  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  function run(args: Record<string, string>): void {
    runApp.mutate(
      { handle, args },
      {
        onSuccess: ({ instanceId }) => {
          onClose();
          void navigate({ to: "/workflows/$instanceId", params: { instanceId } });
        },
      },
    );
  }

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close run"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
        data-testid="app-run-drawer"
        // biome-ignore lint/a11y/useSemanticElements: non-modal side panel; dialog semantics via role+aria-label (mirrors StepConfigDrawer).
        role="dialog"
        aria-label={`Run ${handle}`}
        initial={slideIn.initial}
        animate={slideIn.animate}
        transition={slideIn.transition}
      >
        <div className="node-drawer__head">
          <h3>Run app</h3>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        <div className="node-drawer__section">
          <code className="mono muted">{handle}</code>
          {runApp.isError ? (
            <ErrorNotice error={toUiError(runApp.error)} onRetry={() => runApp.reset()} />
          ) : null}
          {app.isLoading ? (
            <p className="muted">Loading inputs…</p>
          ) : form ? (
            <RecipeForm
              form={form}
              pending={runApp.isPending}
              onSubmit={(args) => {
                // input_schema args fold into the prompt as plain strings.
                const stringified: Record<string, string> = {};
                for (const [k, v] of Object.entries(args)) {
                  stringified[k] = String(v);
                }
                run(stringified);
              }}
            />
          ) : (
            <div className="app-run-drawer__noinputs">
              <p className="muted">This App takes no inputs.</p>
              <button
                type="button"
                className="btn-primary"
                data-testid="app-run-now"
                disabled={runApp.isPending}
                onClick={() => run({})}
              >
                {runApp.isPending ? "Running…" : "Run now"}
              </button>
            </div>
          )}
        </div>
      </m.aside>
    </>
  );
}
