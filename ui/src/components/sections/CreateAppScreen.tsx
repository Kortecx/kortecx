/**
 * The `/apps/create` screen — the whole create journey on ONE surface, through to a
 * TERMINAL result:
 *
 *  1. The compose/review/approve form ({@link NewAppForm}).
 *  2. Once the App is saved and its scaffold launched, the form is replaced by the
 *     live scaffold view ({@link ScaffoldProgress}) — the author stays HERE while
 *     the server writes the project, instead of being dropped on the App page
 *     mid-write with no terminal state to see.
 *  3. When the scaffold reaches done/failed (or failed to LAUNCH at all), the
 *     create-result dialog reports it honestly:
 *       - success ⇒ the app is usable: [Open app] / [Done → back to Apps];
 *       - failure ⇒ the error + what to do: the App is saved as a DRAFT —
 *         [Resume] re-fires the scaffold (deterministic: the committed plan marker
 *         is the truth), [Discard] deletes the draft App, [Close] keeps it for
 *         later (the Draft badge on the Apps page marks it).
 */

import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { toUiError } from "../../kx/errors";
import { useDeleteApp } from "../../kx/use-apps";
import { useScaffoldApp, useScaffoldStatus } from "../../kx/use-scaffold-app";
import { type AppLaunchOutcome, NewAppForm, type NewAppKind } from "./NewAppForm";
import { ScaffoldProgress } from "./ScaffoldProgress";

export function CreateAppScreen({ initialKind = "scheduled" }: { initialKind?: NewAppKind }) {
  const navigate = useNavigate();
  const [launched, setLaunched] = useState<AppLaunchOutcome | null>(null);
  // The user dismissed the terminal dialog with "Close" (keep the draft) — the
  // scaffold view stays visible behind it, so a re-open isn't needed here.
  const [resultDismissed, setResultDismissed] = useState(false);
  const resume = useScaffoldApp();
  const deleteApp = useDeleteApp();

  // Poll the scaffold only once one is running (a launch FAILURE needs no poll —
  // the failure is the result). Polling stops itself on a terminal phase.
  const polling = launched !== null && launched.launchError === null;
  const status = useScaffoldStatus(polling ? launched.handle : null, polling);
  const phase = status.data?.phase;
  const terminal =
    launched === null
      ? null
      : launched.launchError !== null
        ? ("failed" as const)
        : phase === "done" || phase === "failed"
          ? phase
          : null;
  const failDetail =
    launched?.launchError ??
    (phase === "failed" ? status.data?.detail || "the scaffold stopped before finishing" : null);

  function toHome(kind: NewAppKind): void {
    void navigate({
      to: "/apps",
      search: kind === "hosted" ? { section: "hosted" as const } : {},
    });
  }

  return (
    <section className="screen" data-testid="apps-create">
      {launched === null ? (
        <NewAppForm
          initialKind={initialKind}
          onClose={() => void navigate({ to: "/apps" })}
          onLaunched={(outcome) => setLaunched(outcome)}
        />
      ) : launched.launchError === null ? (
        <ScaffoldProgress branchHandle={launched.handle} appHandle={launched.handle} />
      ) : (
        // Launch failed: there is no scaffold to watch; the dialog carries the story.
        <p className="muted" data-testid="apps-create-launch-failed">
          The app was saved, but its project scaffold could not start.
        </p>
      )}

      {launched !== null && terminal !== null && !resultDismissed ? (
        <CreateResultDialog
          outcome={launched}
          failed={terminal === "failed"}
          launchFailed={launched.launchError !== null}
          failDetail={failDetail}
          resuming={resume.isPending}
          discarding={deleteApp.isPending}
          onOpen={() => void navigate({ to: "/apps/$handle", params: { handle: launched.handle } })}
          onDone={() => toHome(launched.kind)}
          onResume={() => {
            // Re-fire the scaffold and go back to watching it. Clearing the launch
            // error re-arms the poll; the server resumes from the committed marker.
            resume.mutate(
              { handle: launched.handle },
              {
                onSuccess: () => {
                  setLaunched({ ...launched, launchError: null });
                  setResultDismissed(false);
                  void status.refetch();
                },
              },
            );
          }}
          resumeError={resume.error ? toUiError(resume.error).message : null}
          onDiscard={() => {
            deleteApp.mutate(
              { handle: launched.handle },
              { onSuccess: () => toHome(launched.kind) },
            );
          }}
          onClose={() => {
            setResultDismissed(true);
            if (terminal === "failed" && launched.launchError !== null) {
              // Nothing to watch behind the dialog — back to the catalog, where
              // the Draft badge marks the app for later.
              toHome(launched.kind);
            }
          }}
        />
      ) : null}
    </section>
  );
}

/** The terminal create-result dialog (the DeleteAppDialog recipe). */
function CreateResultDialog({
  outcome,
  failed,
  launchFailed,
  failDetail,
  resuming,
  discarding,
  resumeError,
  onOpen,
  onDone,
  onResume,
  onDiscard,
  onClose,
}: {
  outcome: AppLaunchOutcome;
  failed: boolean;
  /** The scaffold never LAUNCHED (vs launched and then failed) — the server may
   *  not have marked the draft, so the copy must not promise the badge. */
  launchFailed: boolean;
  failDetail: string | null;
  resuming: boolean;
  discarding: boolean;
  resumeError: string | null;
  onOpen: () => void;
  onDone: () => void;
  onResume: () => void;
  onDiscard: () => void;
  onClose: () => void;
}) {
  const safeRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    safeRef.current?.focus();
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
        aria-label={failed ? "Close the create result" : "Dismiss"}
        onClick={onClose}
      />
      <div className="dialog-center dialog-center--overlay">
        <m.div
          className="dialog-card"
          data-testid="app-create-result"
          data-outcome={failed ? "failed" : "done"}
          // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; modal semantics via role+aria-label (the DeleteAppDialog precedent)
          role="dialog"
          aria-label={failed ? "The app could not be fully created" : "The app is ready"}
          initial={{ y: 12, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          transition={{ type: "spring", stiffness: 420, damping: 34 }}
        >
          {failed ? (
            <>
              <h2 className="dialog-card__title">The scaffold didn't finish</h2>
              <p className="dialog-card__label" data-testid="app-create-result-detail">
                {failDetail ?? "The scaffold stopped before writing every file."}
              </p>
              {launchFailed ? (
                <p className="dialog-card__label">
                  The app is saved. Resume the scaffold to build its project, or discard the app.
                  Closing keeps it for later.
                </p>
              ) : (
                <p className="dialog-card__label">
                  The app is saved as a <strong>draft</strong> — resume to finish the remaining
                  files, or discard it. Closing keeps the draft; it stays marked on the Apps page.
                </p>
              )}
              {resumeError ? (
                <p className="field-error" role="alert" data-testid="app-create-resume-error">
                  {resumeError}
                </p>
              ) : null}
              <div className="dialog-card__actions">
                <button
                  type="button"
                  className="btn-ghost"
                  data-testid="app-create-result-discard"
                  disabled={discarding}
                  onClick={onDiscard}
                >
                  {discarding ? "Discarding…" : "Discard draft"}
                </button>
                <button
                  ref={safeRef}
                  type="button"
                  className="btn-ghost"
                  data-testid="app-create-result-close"
                  onClick={onClose}
                >
                  Close
                </button>
                <button
                  type="button"
                  className="btn-primary"
                  data-testid="app-create-result-resume"
                  disabled={resuming}
                  onClick={onResume}
                >
                  {resuming ? "Resuming…" : "Resume"}
                </button>
              </div>
            </>
          ) : (
            <>
              <h2 className="dialog-card__title">The app is ready</h2>
              <p className="dialog-card__label">
                <code className="mono">{outcome.handle}</code> is created and its project is
                written. It is usable now.
              </p>
              <div className="dialog-card__actions">
                <button
                  type="button"
                  className="btn-ghost"
                  data-testid="app-create-result-done"
                  onClick={onDone}
                >
                  Done
                </button>
                <button
                  ref={safeRef}
                  type="button"
                  className="btn-primary"
                  data-testid="app-create-result-open"
                  onClick={onOpen}
                >
                  Open app
                </button>
              </div>
            </>
          )}
        </m.div>
      </div>
    </>,
    document.body,
  );
}
