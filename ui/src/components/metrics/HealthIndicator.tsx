import { useHealth } from "../../kx/use-health";

/** Gateway liveness as a colored pill (derived from a unary probe; see useHealth). */
export function HealthIndicator() {
  const { data } = useHealth();
  const h = data ?? "live";
  const tone = h === "live" ? "committed" : h === "degraded" ? "scheduled" : "failed";
  return (
    <span className={`pill pill--${tone}`} data-testid="health-indicator" data-health={h}>
      {h.toUpperCase()}
    </span>
  );
}
