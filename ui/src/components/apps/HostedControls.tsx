/**
 * D213 Experience lane — the hosted-app control cluster, shared by the catalog card and
 * the App detail page.
 *
 * Lifted out of `AppsSection` so both surfaces drive the SAME hooks. The detail page
 * previously had no kind check at all: it offered the scheduled-lane Run, which reaches
 * `RunApp` and is refused server-side for a blueprint-less app, while every hosted
 * lifecycle control the catalog already had was missing. Two implementations of "start a
 * hosted app" is how that gap stays open.
 *
 * Testids are suffixed with the handle AND, on the detail page, scoped by a `variant`, so
 * a surface rendering both (or a future one) never makes a `getByTestId` ambiguous.
 */

import { useHostedAppStatus, useHostedRun, useStopHostedApp } from "../../kx/use-hosted-app";
import { Icon } from "../shell/Icon";

/** Where the cluster is rendered — decides icon size and testid prefix. */
export type HostedVariant = "card" | "detail";

const PREFIX: Record<HostedVariant, string> = {
  card: "hosted",
  detail: "app-detail-hosted",
};
const SIZE: Record<HostedVariant, number> = { card: 16, detail: 18 };

/** D213 — a hosted app's live status pill (running / building / starting / stopped / failed). */
export function HostedStatusPill({
  handle,
  variant = "card",
}: {
  handle: string;
  variant?: HostedVariant;
}) {
  const { status } = useHostedAppStatus(handle, true);
  const state = status?.state ?? "stopped";
  const tone =
    state === "running"
      ? "committed"
      : state === "failed"
        ? "failed"
        : state === "stopped"
          ? "unknown"
          : "pending";
  return (
    <span
      className={`pill pill--${tone}`}
      data-testid={`${PREFIX[variant]}-status-${handle}`}
      title={status?.detail || state}
    >
      {state}
    </span>
  );
}

/** D213 — the hosted-app supervisor detail line (install / build / start progress or the
 *  failure reason). It shows the live `HostedAppStatus.detail` while the app is working or
 *  has failed, and stays quiet once it is plainly running or stopped. React Query dedups
 *  the poll with the status pill, so this adds no extra network. */
export function HostedDetail({
  handle,
  variant = "card",
}: {
  handle: string;
  variant?: HostedVariant;
}) {
  const { status } = useHostedAppStatus(handle, true);
  const detail = status?.detail?.trim();
  const state = status?.state ?? "stopped";
  if (!detail || state === "stopped" || state === "running") {
    return null;
  }
  return (
    <p className="card-grid__sub muted" data-testid={`${PREFIX[variant]}-detail-${handle}`}>
      {detail}
    </p>
  );
}

/** D213 — the hosted-app Run control: start the server, open it once actually running,
 *  surface start errors, and honest-disable when the gateway lacks the hosted-apps feature. */
export function HostedRunButton({
  handle,
  variant = "card",
}: {
  handle: string;
  variant?: HostedVariant;
}) {
  const { run, disabled, busy, error } = useHostedRun(handle);
  const size = SIZE[variant];
  if (disabled) {
    return (
      <span
        className="iconbtn iconbtn--disabled"
        aria-disabled="true"
        title="Hosted apps aren't available on this gateway (serve with the hosted-apps feature)"
        data-testid={`${PREFIX[variant]}-run-${handle}`}
      >
        <Icon name="external-link" size={size} />
      </span>
    );
  }
  return (
    <>
      <button
        type="button"
        className="iconbtn"
        data-testid={`${PREFIX[variant]}-run-${handle}`}
        disabled={busy}
        aria-busy={busy}
        title={busy ? "Starting the hosted app…" : "Run this hosted app (opens in a new tab)"}
        aria-label="Run hosted app"
        onClick={run}
      >
        <Icon name="external-link" size={size} />
      </button>
      {error ? (
        <span
          className="hosted-run__error field-error"
          role="alert"
          data-testid={`${PREFIX[variant]}-run-error-${handle}`}
        >
          {error}
        </span>
      ) : null}
    </>
  );
}

/** The hosted-app card cluster: a status pill + the Run-in-new-tab control. */
export function HostedControls({
  handle,
  variant = "card",
}: {
  handle: string;
  variant?: HostedVariant;
}) {
  return (
    <>
      <HostedStatusPill handle={handle} variant={variant} />
      <HostedRunButton handle={handle} variant={variant} />
    </>
  );
}

/**
 * Stop the hosted app's server. Detail-page only.
 *
 * `useStopHostedApp` shipped with the rest of the lane and had ZERO call sites — the
 * console could start a dev server and then had no way to reclaim its port. Disabled
 * (rather than hidden) when nothing is running, so the control's existence is not a
 * function of poll timing.
 */
export function HostedStopButton({ handle }: { handle: string }) {
  const stop = useStopHostedApp();
  const { status, notWired } = useHostedAppStatus(handle, true);
  const state = status?.state ?? "stopped";
  const idle = state === "stopped" || state === "failed";
  if (notWired) {
    return null;
  }
  return (
    <button
      type="button"
      className="iconbtn"
      data-testid={`app-detail-hosted-stop-${handle}`}
      disabled={idle || stop.isPending}
      aria-busy={stop.isPending}
      title={idle ? "Nothing is running" : "Stop this app's server and free its port"}
      aria-label="Stop hosted app"
      onClick={() => stop.mutate({ handle })}
    >
      <Icon name="stop" size={18} />
    </button>
  );
}

/**
 * Restart the app clean: re-materialize, drop `node_modules`, reinstall, restart.
 *
 * This is what the wire's `rebuild` flag actually does — it re-runs whichever lane the
 * app is configured for, and does NOT produce a production build. Labelled for that,
 * because "Rebuild" reads as a build and "Build" would be a lie. The flag was plumbed
 * end-to-end (proto → SDK → hook) and no UI ever set it.
 */
export function HostedRestartButton({ handle }: { handle: string }) {
  const { restart, disabled, busy } = useHostedRun(handle);
  if (disabled) {
    return null;
  }
  return (
    <button
      type="button"
      className="iconbtn"
      data-testid={`app-detail-hosted-restart-${handle}`}
      disabled={busy}
      aria-busy={busy}
      title="Restart clean — re-materialize the project, reinstall dependencies, and restart the server"
      aria-label="Restart hosted app clean"
      onClick={restart}
    >
      <Icon name="refresh" size={18} />
    </button>
  );
}
