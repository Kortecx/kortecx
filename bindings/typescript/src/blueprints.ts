/**
 * The Blueprint builder — author a Tier-1 DAG (a vetted palette of PURE / MODEL
 * steps + DATA/CONTROL edges) for `SubmitWorkflow`. Kept in its own module so
 * `types.ts` stays a thin aggregator (the Rust core's module-per-concern discipline).
 *
 * SN-8: the builder NEVER computes a MoteId or a warrant — it only assembles the
 * topology + params the SERVER compiles + admits. The server assigns each step's
 * logic_ref from its kind and builds every warrant from the party's grants; a
 * tampered client DAG only changes what is PROPOSED, never what identity it gets.
 */

import type { MessageInitShape } from "@bufbuild/protobuf";
import { EdgeKind } from "./gen/kortecx/v1/coordinator_pb.js";
import {
  type SubmitWorkflowRequestSchema,
  WorkflowExecutionMode,
  WorkflowStepKind,
} from "./gen/kortecx/v1/gateway_pb.js";
import { decodeFixed } from "./hexids.js";

/** The vetted step palette (EXEC is reserved server-side in PR-1; TOOL = PR-6b-2). */
export type StepKind = "pure" | "model" | "exec" | "tool";
/** Frozen = memoize/reuse (default); dynamic is reserved server-side in PR-1. */
export type ExecutionMode = "frozen" | "dynamic";
export type EdgeType = "data" | "control";

/** One authored step. `params` values may be a string (UTF-8 encoded) or raw bytes. */
export interface StepInput {
  kind: StepKind;
  modelId?: string;
  prompt?: string;
  /** EXEC only: the registered body's content/signature id as 64-char hex. */
  bodySignatureId?: string;
  toolContract?: Record<string, string>;
  params?: Record<string, Uint8Array | string>;
}

/** One authored edge between two steps (by their `addStep` index). */
export interface EdgeInput {
  parent: number;
  child: number;
  /** `data` (default) | `control`. */
  edge?: EdgeType;
  nonCascade?: boolean;
}

const STEP_KIND: Record<StepKind, WorkflowStepKind> = {
  pure: WorkflowStepKind.PURE,
  model: WorkflowStepKind.MODEL,
  exec: WorkflowStepKind.EXEC,
  tool: WorkflowStepKind.TOOL,
};

function paramBytes(v: Uint8Array | string): Uint8Array {
  return typeof v === "string" ? new TextEncoder().encode(v) : v;
}

/**
 * A fluent builder for a {@link SubmitWorkflowRequest}. `addStep` returns the
 * step's index (use it to wire edges); `build()` produces the wire request.
 */
export class BlueprintBuilder {
  private readonly _steps: StepInput[] = [];
  private readonly _edges: EdgeInput[] = [];
  private _mode: ExecutionMode = "frozen";
  private _contextBundles: string[] = [];

  constructor(private readonly seed: number = 0) {}

  /** Append a step; returns its index (the handle used to wire edges). */
  addStep(step: StepInput): number {
    this._steps.push(step);
    return this._steps.length - 1;
  }

  addEdge(edge: EdgeInput): this {
    this._edges.push(edge);
    return this;
  }

  mode(m: ExecutionMode): this {
    this._mode = m;
    return this;
  }

  /**
   * PR-7: attach context-bundle handles to the run (verbatim order — the SERVER
   * canonicalizes + injects into every entry Mote at bind, SN-8). An empty list ⇒
   * a request byte-identical to pre-PR-7.
   */
  contextBundles(handles: readonly string[]): this {
    this._contextBundles = [...handles];
    return this;
  }

  build(): MessageInitShape<typeof SubmitWorkflowRequestSchema> {
    return {
      seed: this.seed,
      steps: this._steps.map((s) => ({
        kind: STEP_KIND[s.kind],
        modelId: s.modelId ?? "",
        prompt: s.prompt ?? "",
        bodySignatureId: s.bodySignatureId ? decodeFixed(s.bodySignatureId, 32) : new Uint8Array(),
        toolContract: s.toolContract ?? {},
        params: Object.fromEntries(
          Object.entries(s.params ?? {}).map(([k, v]) => [k, paramBytes(v)]),
        ),
      })),
      edges: this._edges.map((e) => ({
        parent: e.parent,
        child: e.child,
        edgeKind: e.edge === "control" ? EdgeKind.CONTROL : EdgeKind.DATA,
        nonCascade: e.nonCascade ?? false,
      })),
      executionMode:
        this._mode === "dynamic" ? WorkflowExecutionMode.DYNAMIC : WorkflowExecutionMode.FROZEN,
      contextBundles: [...this._contextBundles],
    };
  }
}
