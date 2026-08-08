import { Handle, Position } from "@xyflow/react";
import type { NodeProps } from "@xyflow/react";
import { m } from "framer-motion";
import { memo } from "react";
import { statePulse } from "../../app/motion";
import { isTerminalState, stateVisual } from "../../lib/colors";
import { shortHex } from "../../lib/format";
import { STEP_LABEL } from "../../lib/step-kind";
import { branchLabel, observationLabel, turnStepType } from "../../lib/turn-label";
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
  const {
    mote,
    resultContent,
    resultMissing,
    resultLoading,
    swarmRole,
    stepType,
    turnLabel,
    observationOf,
  } = data;
  const { tone } = stateVisual(mote.stateCode);
  const inFlight = !isTerminalState(mote.stateCode);
  // An agent turn names itself the way the Timeline names it — same row, same words,
  // from `lib/turn-label`. Everything else keeps the short Mote id it always had.
  const kind = turnLabel ? turnStepType(turnLabel) : stepType;
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
        {turnLabel ? (
          <span className="dag-node__turn" data-testid="dag-node-turn" title={mote.moteId}>
            Turn {turnLabel.turn}
          </span>
        ) : observationOf ? (
          // The observation of a turn names ITS turn — but it is NOT that turn, and it
          // must not present as a second card headed `Turn 1`. It carries its own
          // testid (a live run caught two nodes answering to the same turn, and the
          // surface that has to agree with the Timeline picked whichever came last)
          // and says `result` in the head, so the pair reads as cause and effect.
          <span className="dag-node__turn" data-testid="dag-node-observation" title={mote.moteId}>
            Turn {observationOf.turn} · result
          </span>
        ) : (
          <span className="dag-node__id mono" title={mote.moteId}>
            {shortHex(mote.moteId)}
          </span>
        )}
        {swarmRole === "gather" ? (
          <span className="chip chip--static dag-node__role">gather</span>
        ) : null}
      </div>
      <div className="dag-node__row">
        <StatePill stateCode={mote.stateCode} />
        {/* The determinism class is runtime machinery, and on a turn node the turn's own
            branch says more to a reader. Same for an OBSERVATION: `WORLD_MUTATING` beside
            a hash was the whole of what an echo result said about itself. It stays on
            every other Mote, and the full detail is one click away in the drawer. */}
        {turnLabel || observationOf ? null : <NdClassBadge ndClass={mote.ndClass} />}
        {kind ? (
          <span
            className={`chip chip--static dag-node__step dag-node__step--${kind}`}
            data-testid="dag-node-step"
            data-step={kind}
          >
            {STEP_LABEL[kind]}
          </span>
        ) : null}
      </div>
      {turnLabel ? (
        <div
          className="dag-node__branch"
          data-testid="dag-node-branch"
          data-branch={turnLabel.branch}
        >
          {branchLabel(turnLabel)}
        </div>
      ) : observationOf ? (
        <div className="dag-node__branch" data-testid="dag-node-branch" data-branch="observation">
          {observationLabel(observationOf)}
        </div>
      ) : null}
      {mote.resultRef ? (
        <div className="dag-node__result">
          {/* The resolved text glimpse; the chip + full result live in the
              click→drawer (a chip button here would bubble to the node click). */}
          <ResultPreview
            resultRef={mote.resultRef}
            content={resultContent}
            missing={resultMissing}
            loading={resultLoading}
            // 64 characters is barely past the opening brace of a JSON result — the
            // node showed the shape and none of the answer. The card is two lines of
            // preview wide; this fills them.
            max={96}
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
