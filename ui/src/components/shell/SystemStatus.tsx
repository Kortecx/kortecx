import { type Health, useHealth } from "../../kx/use-health";

/**
 * The topbar system-status box (PR-B / D150). Adopts the reference app's status
 * pill HONESTLY: gateway liveness is REAL (`useHealth` polls `listSignatures`), so
 * we show the live/degraded/down tone + a pulse dot + the word. The reference's
 * "active agents" + "uptime" have NO backing RPC, so they are OMITTED rather than
 * fabricated (GR15 / D142.4 telemetry-first). Liveness mirrors `ConnectionStatus`
 * (live default; an unreachable gateway resolves to `down`).
 */
const TONE: Record<Health, string> = {
  live: "status-dot--online",
  degraded: "status-dot--busy",
  down: "status-dot--error",
};
const WORD: Record<Health, string> = { live: "Live", degraded: "Degraded", down: "Down" };

export function SystemStatus() {
  const health = useHealth();
  const h: Health = health.data ?? "live";
  return (
    <span className="status-box" data-testid="system-status" data-health={h} title={`Gateway ${h}`}>
      <span
        className={`status-dot ${TONE[h]}${h === "live" ? " status-dot--pulse" : ""}`}
        aria-hidden="true"
      />
      <span className="status-box__label">{WORD[h]}</span>
    </span>
  );
}
