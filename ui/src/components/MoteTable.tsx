import { memo, useMemo } from "react";
import { useResultMap } from "../kx/use-content-batch";
import type { ProjectionVM } from "../kx/use-projection";
import { EmptyState } from "./EmptyState";
import { MoteRow } from "./MoteRow";

/**
 * The run's Motes as a status table. The Result column shows the RESOLVED text
 * (the headline) with a digest chip (D142.2) — every committed result is fetched
 * in ONE `getContentBatch` via `useResultMap` (the N+1 collapse), so the whole
 * table resolves with a single round trip. Memoized so an unchanged poll (stable
 * `projection` reference via structural sharing) is a no-op.
 */
function MoteTableImpl({ projection }: { projection: ProjectionVM }) {
  // Batch-resolve every committed result in the visible projection (one RPC).
  // The run scope rides the projection (no separate prop to thread, so the DAG's
  // >MAX table fallback resolves text too).
  const refs = useMemo(
    () => projection.motes.flatMap((m) => (m.resultRef ? [m.resultRef] : [])),
    [projection.motes],
  );
  const { byRef, isLoading } = useResultMap(projection.instanceId, refs);

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
      {/* Fixed column widths so the Result column is BOUNDED — its resolved text
          ellipsis-clips inside the cell and the trailing digest chip stays on
          screen (an unbounded Result column pushes the chip past the viewport). */}
      <colgroup>
        <col style={{ width: "13%" }} />
        <col style={{ width: "11%" }} />
        <col style={{ width: "9%" }} />
        <col style={{ width: "11%" }} />
        <col style={{ width: "9%" }} />
        <col style={{ width: "37%" }} />
        <col style={{ width: "10%" }} />
      </colgroup>
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
        {projection.motes.map((m) => {
          const vm = m.resultRef ? byRef.get(m.resultRef) : undefined;
          return (
            <MoteRow
              key={m.moteId}
              mote={m}
              content={vm?.content}
              missing={vm?.missing ?? false}
              resolving={isLoading}
            />
          );
        })}
      </tbody>
    </table>
  );
}

export const MoteTable = memo(MoteTableImpl);
