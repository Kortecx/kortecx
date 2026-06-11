import { Handle, Position } from "@xyflow/react";
import type { NodeProps } from "@xyflow/react";
import { m } from "framer-motion";
import { memo } from "react";
import { statePulse } from "../../app/motion";
import { stateVisual } from "../../lib/colors";
import { shortHex } from "../../lib/format";
import { AnomalyBadge } from "../AnomalyBadge";
import { NdClassBadge } from "../NdClassBadge";
import { StatePill } from "../StatePill";
import type { MoteFlowNode } from "./flow";

/**
 * One Mote as a DAG node: id + state pill + nd_class badge (+ anomaly). Reuses
 * the table's visual vocabulary (`StatePill`/`NdClassBadge`/`stateVisual`) so the
 * two surfaces never diverge. A newly-mounted node (a dynamic shaper child) plays
 * the one-shot enter pulse; persistent nodes keep their instance (no re-pulse).
 */
function MoteNodeImpl({ data }: NodeProps<MoteFlowNode>) {
  const { mote } = data;
  const { tone } = stateVisual(mote.stateCode);
  return (
    <m.div
      className={`dag-node dag-node--${tone}`}
      data-testid="mote-node"
      data-mote={mote.moteId}
      data-state={mote.stateCode}
      initial={statePulse.initial}
      animate={statePulse.animate}
      transition={statePulse.transition}
      aria-label={`Mote ${shortHex(mote.moteId)}`}
    >
      <Handle type="target" position={Position.Top} className="dag-handle" />
      <div className="dag-node__id mono" title={mote.moteId}>
        {shortHex(mote.moteId)}
      </div>
      <div className="dag-node__row">
        <StatePill stateCode={mote.stateCode} />
        <NdClassBadge ndClass={mote.ndClass} />
      </div>
      <AnomalyBadge anomaly={mote.anomaly} />
      <Handle type="source" position={Position.Bottom} className="dag-handle" />
    </m.div>
  );
}

export const MoteNode = memo(MoteNodeImpl);
