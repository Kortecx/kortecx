import { useConnection } from "../../kx/connection-context";
import { useHealth } from "../../kx/use-health";

/** Connection + liveness pill: a dot (live/degraded/down), endpoint, disconnect. */
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
  const dotClass = h === "live" ? "dot--ok" : h === "degraded" ? "dot--degraded" : "dot--off";
  return (
    <span className="connstatus" data-testid="conn-status" data-status={status} data-health={h}>
      <span className={`dot ${dotClass}`} title={`gateway ${h}`} />
      <span className="mono connstatus__ep" title={endpoint}>
        {endpoint}
      </span>
      <button type="button" className="linkbtn" onClick={disconnect}>
        disconnect
      </button>
    </span>
  );
}
