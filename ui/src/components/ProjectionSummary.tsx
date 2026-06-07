import type { ProjectionVM } from "../kx/use-projection";
import { countSummary, formatSeq, shortHex } from "../lib/format";

const COMMITTED = 3; // MoteSnapshotState.COMMITTED

export function ProjectionSummary({
  projection,
  polling,
}: {
  projection: ProjectionVM;
  polling: boolean;
}) {
  const committed = projection.motes.filter((m) => m.stateCode === COMMITTED).length;
  const total = projection.motes.length;
  return (
    <div className="proj-summary" data-testid="projection-summary">
      <span>seq {formatSeq(projection.currentSeq)}</span>
      <span className="mono" title={projection.recipeFingerprint}>
        recipe {shortHex(projection.recipeFingerprint)}
      </span>
      <span>{countSummary(committed, total, "committed")}</span>
      {polling ? (
        <span className="pulse" aria-live="polite" data-testid="polling-indicator">
          ● live
        </span>
      ) : (
        <span data-testid="at-rest-indicator">○ at rest</span>
      )}
    </div>
  );
}
