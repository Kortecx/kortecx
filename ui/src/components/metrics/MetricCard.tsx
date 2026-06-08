import type { ReactNode } from "react";

/** One labelled stat. `tone` tints the value (reuses the `--t-*` palette). */
export function MetricCard({
  label,
  value,
  tone,
}: {
  label: string;
  value: ReactNode;
  tone?: string;
}) {
  return (
    <div
      className={tone ? `metric-card metric-card--${tone}` : "metric-card"}
      data-testid="metric-card"
    >
      <span className="metric-card__value">{value}</span>
      <span className="metric-card__label">{label}</span>
    </div>
  );
}
