import { m } from "framer-motion";
import type { ReactNode } from "react";
import { fadeUp } from "../../app/motion";

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
    <m.div
      className={tone ? `metric-card metric-card--${tone}` : "metric-card"}
      data-testid="metric-card"
      variants={fadeUp}
    >
      <span className="metric-card__value">{value}</span>
      <span className="metric-card__label">{label}</span>
    </m.div>
  );
}
