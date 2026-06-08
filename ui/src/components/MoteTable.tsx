import { memo } from "react";
import type { ProjectionVM } from "../kx/use-projection";
import { EmptyState } from "./EmptyState";
import { MoteRow } from "./MoteRow";

/**
 * The run's Motes as a status table. This is the T3.3 forward seam: the live DAG
 * viewer will replace this view (keeping the same `ProjectionVM` input). Memoized so
 * an unchanged poll (stable `projection` reference via structural sharing) is a no-op.
 */
function MoteTableImpl({ projection }: { projection: ProjectionVM }) {
  if (projection.motes.length === 0) {
    return (
      <EmptyState
        title="No Motes yet"
        detail="This run has no Motes at the current frontier — they appear as the run executes."
      />
    );
  }
  return (
    <table className="mote-table" data-testid="mote-table">
      <thead>
        <tr>
          <th scope="col">Mote</th>
          <th scope="col">State</th>
          <th scope="col">nd_class</th>
          <th scope="col">Promotion</th>
          <th scope="col">Committed</th>
          <th scope="col">Result</th>
          <th scope="col">Anomaly</th>
        </tr>
      </thead>
      <tbody>
        {projection.motes.map((m) => (
          <MoteRow key={m.moteId} mote={m} />
        ))}
      </tbody>
    </table>
  );
}

export const MoteTable = memo(MoteTableImpl);
