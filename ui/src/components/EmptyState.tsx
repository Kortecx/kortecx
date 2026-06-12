import { m } from "framer-motion";
import { emptyState } from "../app/motion";

export interface EmptyStateProps {
  title: string;
  detail?: string;
  /** An optional actionable affordance (a retry/primary-action button) — the
   *  D142.3 "what to do next" line, rendered under the detail copy. */
  action?: React.ReactNode;
}

/** A neutral empty/placeholder panel. */
export function EmptyState({ title, detail, action }: EmptyStateProps) {
  return (
    <m.div className="empty-state" data-testid="empty-state" {...emptyState}>
      <p className="empty-state__title">{title}</p>
      {detail ? <p className="empty-state__detail">{detail}</p> : null}
      {action ? <div className="empty-state__action">{action}</div> : null}
    </m.div>
  );
}
