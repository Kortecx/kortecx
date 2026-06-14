import { Handle, Position } from "@xyflow/react";
import type { Node, NodeProps } from "@xyflow/react";
import { memo } from "react";
import type { BuilderStep } from "./builder-graph";

/** A reactflow node's data payload in the builder (the authored step). The index
 *  signature satisfies reactflow's `Node<T>`. */
export interface BuilderNodeData extends Record<string, unknown> {
  readonly step: BuilderStep;
}

/** The builder's reactflow node type. */
export type BuilderFlowNode = Node<BuilderNodeData, "builder">;

/**
 * One authored step as an EDITABLE builder node (the rich-graph counterpart of the
 * read-only `MoteNode`, D141.3). Reuses the `.dag-node` design language with a
 * builder accent; draggable + connectable (the canvas enables it). A MODEL step
 * with no model — or a step with invalid params — surfaces a visible "needs config"
 * cue (D142 every-state-designed) rather than failing silently at submit.
 */
function BuilderNodeImpl({ data, selected }: NodeProps<BuilderFlowNode>) {
  const { step } = data;
  const tone = step.kind === "model" ? "model" : "pure";
  const needsConfig = step.kind === "model" && step.modelId.trim() === "";
  return (
    <div
      className={`dag-node builder-node builder-node--${tone}${selected ? " builder-node--selected" : ""}`}
      data-testid="builder-node"
      data-node={step.id}
      data-kind={step.kind}
      aria-label={`${step.kind === "model" ? "Agent" : "Step"} ${step.label}`}
    >
      <span className="dag-node__accent" aria-hidden="true" />
      <Handle type="target" position={Position.Top} className="dag-handle" />
      <div className="dag-node__head">
        <span className={`builder-node__kind builder-node__kind--${tone}`}>
          {step.kind === "model" ? "Agent" : "Pure"}
        </span>
        <span className="builder-node__label" title={step.label}>
          {step.label}
        </span>
      </div>
      {step.kind === "model" ? (
        <div className="builder-node__row">
          {needsConfig ? (
            <span className="builder-node__hint" data-testid="builder-node-needs-config">
              pick a model
            </span>
          ) : (
            <span className="builder-node__model mono" title={step.modelId}>
              {step.modelId}
            </span>
          )}
        </div>
      ) : null}
      <Handle type="source" position={Position.Bottom} className="dag-handle" />
    </div>
  );
}

export const BuilderNode = memo(BuilderNodeImpl);
