import { m } from "framer-motion";
import type { ReactNode } from "react";
import { fadeUp } from "../../app/motion";

/**
 * One labelled stat. `tone` tints the value + accent bar (reuses the `--t-*` /
 * semantic palette); `sub` is an optional honest qualifier (e.g. "over the last N
 * motes") rendered under the label in a muted tier — never the headline. The accent
 * bar is decorative, so a tone never puts text on an accent colour (the AA lock).
 */
export function MetricCard({
  label,
  value,
  tone,
  sub,
}: {
  label: string;
  value: ReactNode;
  tone?: string;
  sub?: ReactNode;
}) {
  return (
    <m.div
      className={tone ? `metric-card metric-card--${tone}` : "metric-card"}
      data-testid="metric-card"
      variants={fadeUp}
    >
      <span className="metric-card__value">{value}</span>
      <span className="metric-card__label">{label}</span>
      {sub ? <span className="metric-card__sub">{sub}</span> : null}
    </m.div>
  );
}
