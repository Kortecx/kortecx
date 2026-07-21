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
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import { useApp, useRunApp } from "../../kx/use-apps";
import { appInputForm } from "../../lib/app-input-schema";
import { runViewSearch } from "../../lib/run-anchor";
import { ErrorNotice } from "../ErrorNotice";
import { RecipeForm } from "../recipes/RecipeForm";
import { RunPreflight } from "./RunPreflight";

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
  // Opt-in per-run HITL: when on, the run authors under the approval gate so a
  // world-mutating tool call surfaces in the approvals inbox before it fires.
  const [requireApproval, setRequireApproval] = useState(false);

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
    // Send the opt-in HITL flag only when the user checked it (mirrors the `args`
    // idiom) — an unchecked run keeps today's payload, so the server default applies.
    runApp.mutate(requireApproval ? { handle, args, requireApproval } : { handle, args }, {
      onSuccess: (started) => {
        onClose();
        // Carry the per-submission ANCHOR into the run view. Without it the view falls
        // back to the whole journal: a serve is ONE journal with ONE instance_id shared by
        // every run, so `/workflows/<instanceId>` alone cannot mean "this run".
        //
        // This used to send the chain salt alone, which meant ▶ on an App — the single
        // most-travelled path to a run view — landed UNSCOPED for every ordinary App. The
        // salt is emitted only for exactly one tool-granted agentic step, and a plain
        // scheduled App is not that shape, so the condition was false almost always.
        // `runViewSearch` prefers the salt and falls back to the terminal Mote, which the
        // server populates for every shape.
        void navigate({
          to: "/workflows/$instanceId",
          params: { instanceId: started.instanceId },
          search: runViewSearch(started),
        });
      },
    });
  }

  return createPortal(
    <>
      <button
        type="button"
        className="node-drawer__scrim node-drawer__scrim--overlay"
        aria-label="Close run"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer node-drawer--overlay"
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
          <RunPreflight handle={handle} />
          <label className="app-run-drawer__approval">
            <input
              type="checkbox"
              data-testid="app-run-require-approval"
              checked={requireApproval}
              onChange={(e) => setRequireApproval(e.target.checked)}
              disabled={runApp.isPending}
            />{" "}
            Require approval before irreversible actions
          </label>
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
    </>,
    document.body,
  );
}
