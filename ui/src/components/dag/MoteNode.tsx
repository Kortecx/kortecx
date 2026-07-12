import { Handle, Position } from "@xyflow/react";
import type { NodeProps } from "@xyflow/react";
import { m } from "framer-motion";
import { memo } from "react";
import { statePulse } from "../../app/motion";
import { isTerminalState, stateVisual } from "../../lib/colors";
import { shortHex } from "../../lib/format";
import { AnomalyBadge } from "../AnomalyBadge";
import { NdClassBadge } from "../NdClassBadge";
import { ResultPreview } from "../ResultPreview";
import { StatePill } from "../StatePill";
import type { MoteFlowNode } from "./flow";

/**
 * One Mote as a DAG node, in the reference design language: a top accent bar + a
 * status dot (pulsing while in-flight) + the short id, then the state/nd_class pills
 * (+ anomaly). Reuses the table's visual vocabulary (`StatePill`/`NdClassBadge`/
 * `stateVisual`) so the two surfaces never diverge. A newly-mounted node (a dynamic
 * shaper child) plays the one-shot enter pulse; persistent nodes keep their instance.
 * The whole card is clickable (reactflow `onNodeClick` opens the detail drawer).
 */
function MoteNodeImpl({ data }: NodeProps<MoteFlowNode>) {
  const { mote, resultContent, resultMissing, resultLoading, swarmRole } = data;
  const { tone } = stateVisual(mote.stateCode);
  const inFlight = !isTerminalState(mote.stateCode);
  return (
    <m.div
      className={`dag-node dag-node--${tone}${swarmRole ? " dag-node--swarm" : ""}`}
      data-testid="mote-node"
      data-mote={mote.moteId}
      data-state={mote.stateCode}
      data-swarm-role={swarmRole}
      initial={statePulse.initial}
      animate={statePulse.animate}
      transition={statePulse.transition}
      aria-label={`Mote ${shortHex(mote.moteId)}`}
    >
      <span className="dag-node__accent" aria-hidden="true" />
      <Handle type="target" position={Position.Top} className="dag-handle" />
      <div className="dag-node__head">
        <span
          className={`dag-node__dot${inFlight ? " dag-node__dot--pulse" : ""}`}
          aria-hidden="true"
        />
        <span className="dag-node__id mono" title={mote.moteId}>
          {shortHex(mote.moteId)}
        </span>
        {swarmRole === "gather" ? (
          <span className="chip chip--static dag-node__role">gather</span>
        ) : null}
      </div>
      <div className="dag-node__row">
        <StatePill stateCode={mote.stateCode} />
        <NdClassBadge ndClass={mote.ndClass} />
      </div>
      {mote.resultRef ? (
        <div className="dag-node__result">
          {/* The resolved text glimpse; the chip + full result live in the
              click→drawer (a chip button here would bubble to the node click). */}
          <ResultPreview
            resultRef={mote.resultRef}
            content={resultContent}
            missing={resultMissing}
            loading={resultLoading}
            max={64}
            chip={false}
          />
        </div>
      ) : null}
      <AnomalyBadge anomaly={mote.anomaly} />
      <Handle type="source" position={Position.Bottom} className="dag-handle" />
    </m.div>
  );
}

export const MoteNode = memo(MoteNodeImpl);
