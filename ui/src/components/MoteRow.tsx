import { memo } from "react";
import type { MoteVM } from "../kx/use-projection";
import { promotionIsNotable, promotionLabel } from "../lib/colors";
import type { DecodedContent } from "../lib/content-decode";
import { formatSeq, shortHex } from "../lib/format";
import { AnomalyBadge } from "./AnomalyBadge";
import { NdClassBadge } from "./NdClassBadge";
import { ResultPreview } from "./ResultPreview";
import { StatePill } from "./StatePill";

function MoteRowImpl({
  mote,
  content,
  missing = false,
  resolving = false,
}: {
  mote: MoteVM;
  /** The resolved result (from the table's batch fetch); undefined while resolving. */
  content?: DecodedContent;
  missing?: boolean;
  resolving?: boolean;
}) {
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
      <td className="mote-table__result">
        {/* max=4096 (the preview-byte cap) means JS does NOT pre-clip — the text
            grows to fill the cell and CSS ellipsis-clips at the chip, so the chip
            sits flush against the text with no gap. The full text is in `title`. */}
        <ResultPreview
          resultRef={mote.resultRef}
          content={content}
          missing={missing}
          loading={resolving}
          max={4096}
        />
      </td>
      <td>
        <AnomalyBadge anomaly={mote.anomaly} />
      </td>
    </tr>
  );
}

// Re-render a row only when a field that affects its display changes. With the
// data layer's structural sharing the prop reference is usually already stable;
// this comparator is the belt-and-braces guarantee the perf budget relies on.
// `content`/`missing`/`resolving` ride the batch fetch — compare them too so a
// row repaints from "resolving…" to its resolved text.
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
    a.anomaly === b.anomaly &&
    prev.content === next.content &&
    prev.missing === next.missing &&
    prev.resolving === next.resolving
  );
});
