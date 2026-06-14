import { useTelemetry } from "../../kx/use-telemetry";

/**
 * The sidebar footer token readout (PR-B / D150). Adopts the reference app's
 * "Daily Token Usage" affordance HONESTLY (GR15 / D142.4): the runtime exposes
 * per-mote `outputTokens` via `ListMoteTelemetry` (set only on inference builds;
 * `inputTokens` is never populated in OSS and there is NO quota/limit RPC). So we
 * show a REAL output-token readout for today — output-only, with NO fabricated
 * limit/bar — and an honest-empty caption when there is no model telemetry.
 *
 * Rendered only when the sidebar is expanded (the caller gates on `collapsed`), so
 * the telemetry poll never runs on the icon rail.
 */
function startOfTodayMs(): number {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

export function TokenUsageFooter() {
  const tel = useTelemetry();
  const since = startOfTodayMs();
  const total = tel.rows.reduce(
    (sum, r) => (r.startedUnixMs >= since ? sum + (r.outputTokens ?? 0) : sum),
    0,
  );
  const hasReal = !tel.notWired && total > 0;

  return (
    <div className="sidebar__footer" data-testid="token-usage">
      <p className="sidebar__footer-label">Output tokens · today</p>
      {hasReal ? (
        <p className="sidebar__footer-value mono">{total.toLocaleString()}</p>
      ) : (
        <p className="sidebar__footer-empty" data-testid="token-usage-empty">
          {tel.notWired ? "— no usage telemetry" : "— no model telemetry"}
        </p>
      )}
    </div>
  );
}
