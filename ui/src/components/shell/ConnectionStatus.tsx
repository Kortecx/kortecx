import { useConnection } from "../../kx/connection-context";
import { useHealth } from "../../kx/use-health";
import { Icon } from "./Icon";

/**
 * Connection control: a single POWER button (user-directed 2026-06-12 — the
 * endpoint text + "disconnect" link left the navbar). The button carries the
 * liveness as its color (live/degraded/down); the endpoint + health stay
 * discoverable on hover (title). Clicking disconnects (back to the login gate).
 */
export function ConnectionStatus() {
  const { status, endpoint, disconnect } = useConnection();
  const health = useHealth();

  if (status !== "connected") {
    return (
      <span className="connstatus" data-testid="conn-status" data-status={status}>
        <span className="dot dot--off" />
        not connected
      </span>
    );
  }

  const h = health.data ?? "live";
  const tone =
    h === "live" ? "var(--success)" : h === "degraded" ? "var(--warning)" : "var(--error)";
  return (
    <span className="connstatus" data-testid="conn-status" data-status={status} data-health={h}>
      <button
        type="button"
        className="iconbtn connstatus__power"
        style={{ color: tone }}
        onClick={disconnect}
        title={`gateway ${h} · ${endpoint} — disconnect`}
        aria-label="Disconnect"
        data-testid="disconnect-btn"
      >
        <Icon name="power" />
      </button>
    </span>
  );
}
