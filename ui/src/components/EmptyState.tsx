import { m } from "framer-motion";
import { emptyState } from "../app/motion";

export interface EmptyStateProps {
  title: string;
  detail?: string;
}

/** A neutral empty/placeholder panel. */
export function EmptyState({ title, detail }: EmptyStateProps) {
  return (
    <m.div className="empty-state" data-testid="empty-state" {...emptyState}>
      <p className="empty-state__title">{title}</p>
      {detail ? <p className="empty-state__detail">{detail}</p> : null}
    </m.div>
  );
}
