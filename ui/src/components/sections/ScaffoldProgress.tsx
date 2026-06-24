/**
 * POC-5a: the honest, polled scaffold-progress panel. After "New App" launches a
 * server-side agentic scaffold, this polls `GetScaffoldStatus` and renders one
 * row per FIXED skeleton path — ✓ done / spinner writing / dot pending — driven
 * ENTIRELY by the server's real `filesDone` / `filesPending` + phase (GR15: never
 * a timer). On `failed` it shows the server `detail` and KEEPS the partial files
 * visible (no fabricated "done"); on `done` it offers an "Open" CTA into the App
 * detail view. Polling stops the instant the phase is terminal.
 */

import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect } from "react";
import { fadeUp, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useInvalidateOnScaffoldDone, useScaffoldStatus } from "../../kx/use-scaffold-app";
import {
  SKELETON_PATHS,
  type ScaffoldRowState,
  deriveScaffoldStatus,
} from "../../lib/scaffold-status";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";

const STATE_GLYPH: Record<ScaffoldRowState, string> = {
  done: "✓",
  writing: "◐",
  pending: "·",
};

const STATE_LABEL: Record<ScaffoldRowState, string> = {
  done: "written",
  writing: "writing…",
  pending: "pending",
};

export function ScaffoldProgress({
  branchHandle,
  appHandle,
}: {
  /** The App's project branch handle to poll (== the App handle by convention). */
  branchHandle: string;
  /** The App handle to navigate to on completion. */
  appHandle: string;
}) {
  const navigate = useNavigate();
  const invalidate = useInvalidateOnScaffoldDone();
  // Poll while a phase is non-terminal; the query halts its own interval on
  // done/failed, and we flip `enabled` off so it never re-arms.
  const status = useScaffoldStatus(branchHandle, true);
  const data = status.data;
  const derived = data ? deriveScaffoldStatus(SKELETON_PATHS, data) : null;

  // On completion, refresh the catalog + branch caches so the new tree appears.
  useEffect(() => {
    if (derived?.complete) {
      invalidate(branchHandle);
    }
  }, [derived?.complete, branchHandle, invalidate]);

  if (status.isLoading) {
    return <EmptyState title="Starting the scaffold…" />;
  }
  if (status.isError) {
    return <ErrorNotice error={toUiError(status.error)} onRetry={() => void status.refetch()} />;
  }
  if (!derived || !data) {
    return null;
  }

  const heading =
    derived.phase === "planning"
      ? "Planning the project…"
      : derived.phase === "writing"
        ? "Writing the project files…"
        : derived.failed
          ? "Scaffold failed"
          : derived.complete
            ? "Project ready"
            : "Scaffolding…";

  return (
    <m.div
      className="scaffold-progress"
      data-testid="scaffold-progress"
      data-phase={derived.phase}
      variants={fadeUp}
      initial="hidden"
      animate="show"
    >
      <div className="scaffold-progress__head">
        <h3 className="scaffold-progress__title">{heading}</h3>
        <code className="mono scaffold-progress__handle" title={appHandle}>
          {appHandle}
        </code>
      </div>

      <m.ul
        className="scaffold-progress__rows"
        variants={stagger(0.04)}
        initial="hidden"
        animate="show"
      >
        {derived.rows.map((row) => (
          <m.li
            key={row.path}
            className="scaffold-progress__row"
            data-testid={`scaffold-row-${row.path}`}
            data-state={row.state}
            variants={fadeUp}
          >
            <span
              className={`scaffold-progress__glyph scaffold-progress__glyph--${row.state}`}
              aria-hidden="true"
            >
              {STATE_GLYPH[row.state]}
            </span>
            <code className="mono scaffold-progress__path">{row.path}</code>
            <span className="muted scaffold-progress__state">{STATE_LABEL[row.state]}</span>
          </m.li>
        ))}
      </m.ul>

      {derived.failed ? (
        <p className="field-error" data-testid="scaffold-failed" role="alert">
          The scaffold did not finish:{" "}
          {data.detail || "the agent stopped without writing every file."} The files written so far
          are kept above — re-run "New App" to resume.
        </p>
      ) : null}

      {derived.complete ? (
        <div className="scaffold-progress__done">
          <p className="muted" data-testid="scaffold-done">
            All skeleton files are written. Open the App to browse and edit its project tree.
          </p>
          <button
            type="button"
            className="btn-primary"
            data-testid="scaffold-open"
            onClick={() => void navigate({ to: "/apps/$handle", params: { handle: appHandle } })}
          >
            Open App
          </button>
        </div>
      ) : null}
    </m.div>
  );
}
