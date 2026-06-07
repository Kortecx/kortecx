import { memo } from "react";
import type { MoteVM } from "../kx/use-projection";
import { promotionIsNotable, promotionLabel } from "../lib/colors";
import { formatSeq, shortHex } from "../lib/format";
import { AnomalyBadge } from "./AnomalyBadge";
import { NdClassBadge } from "./NdClassBadge";
import { StatePill } from "./StatePill";

function MoteRowImpl({ mote }: { mote: MoteVM }) {
  return (
    <tr data-testid="mote-row" data-mote={mote.moteId} data-state={mote.stateCode}>
      <td className="mono" title={mote.moteId}>
        {shortHex(mote.moteId)}
      </td>
      <td>
        <StatePill stateCode={mote.stateCode} />
      </td>
      <td>
        <NdClassBadge ndClass={mote.ndClass} />
      </td>
      <td>{promotionIsNotable(mote.promotion) ? promotionLabel(mote.promotion) : ""}</td>
      <td className="num">{formatSeq(mote.committedSeq)}</td>
      <td className="mono">{mote.resultRef ? shortHex(mote.resultRef) : ""}</td>
      <td>
        <AnomalyBadge anomaly={mote.anomaly} />
      </td>
    </tr>
  );
}

// Re-render a row only when a field that affects its display changes. With the
// data layer's structural sharing the prop reference is usually already stable;
// this comparator is the belt-and-braces guarantee the perf budget relies on.
export const MoteRow = memo(MoteRowImpl, (prev, next) => {
  const a = prev.mote;
  const b = next.mote;
  return (
    a.moteId === b.moteId &&
    a.stateCode === b.stateCode &&
    a.ndClass === b.ndClass &&
    a.promotion === b.promotion &&
    a.resultRef === b.resultRef &&
    a.committedSeq === b.committedSeq &&
    a.anomaly === b.anomaly
  );
});
