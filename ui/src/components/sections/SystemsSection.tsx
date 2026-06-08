import { useConnection } from "../../kx/connection-context";
import { EmptyState } from "../EmptyState";
import { HealthIndicator } from "../metrics/HealthIndicator";

/**
 * The connected gateway at a glance. Agentic systems (teams / `MembershipLedger`)
 * and sharing (grants / `GrantLedger`) read-only VIEWERS arrive in UI-3 (managing
 * across parties stays cloud, per D129).
 */
export function SystemsSection() {
  const { endpoint, wsEndpoint } = useConnection();
  return (
    <section className="screen" data-testid="systems-section">
      <h1>Systems</h1>
      <p className="muted">The connected gateway, its liveness, and (soon) your teams.</p>
      <dl className="facts">
        <dt>Gateway</dt>
        <dd className="mono">{endpoint}</dd>
        <dt>WS bridge</dt>
        <dd className="mono">{wsEndpoint ?? "(derived from endpoint, :50152)"}</dd>
        <dt>Health</dt>
        <dd>
          <HealthIndicator />
        </dd>
      </dl>
      <EmptyState
        title="Teams & sharing"
        detail="Teams (MembershipLedger) and sharing (GrantLedger) viewers arrive in UI-3 — read-only in OSS; managing across parties is cloud."
      />
    </section>
  );
}
