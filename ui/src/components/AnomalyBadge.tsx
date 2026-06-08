import { anomalyLabel } from "../lib/colors";

/** A Mote anomaly warning (renders nothing for a healthy Mote). */
export function AnomalyBadge({ anomaly }: { anomaly: number | null }) {
  const label = anomalyLabel(anomaly);
  if (label === null) {
    return null;
  }
  return (
    <output className="badge badge--anomaly" data-testid="anomaly-badge" title="Mote anomaly">
      ⚠ {label}
    </output>
  );
}
