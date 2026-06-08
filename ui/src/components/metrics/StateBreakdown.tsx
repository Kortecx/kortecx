import type { StateTone } from "../../lib/colors";
import type { Metrics } from "../../lib/metrics";

// Render order (committed first → most prominent), tones reuse the pill palette.
const ORDER: readonly StateTone[] = [
  "committed",
  "scheduled",
  "pending",
  "failed",
  "repudiated",
  "inconsistent",
  "unknown",
];

/** A single proportional bar of Mote states (each segment a `--t-*` tone). */
export function StateBreakdown({ metrics }: { metrics: Metrics }) {
  if (metrics.total === 0) {
    return null;
  }
  return (
    <div className="state-bar" data-testid="state-breakdown" aria-label="Mote state breakdown">
      {ORDER.map((tone) => {
        const n = metrics.byState[tone];
        if (n === 0) {
          return null;
        }
        const pct = (n / metrics.total) * 100;
        return (
          <span
            key={tone}
            className={`state-bar__seg pill--${tone}`}
            style={{ width: `${pct}%` }}
            title={`${tone}: ${n}`}
            data-tone={tone}
          />
        );
      })}
    </div>
  );
}
