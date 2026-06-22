/**
 * The ReAct-chain turn view — one `ReactRound` fact enumerated by `ListReactTurns`
 * (PR-2d-1/2): the durable, queryable Reason→Act→Observe history of a live ReAct
 * chain in `kx serve`. Each turn carries its run-salted Mote id, its settled
 * branch (`pending` | `answer` | `tool` | `rejected` | `dead_lettered`) and —
 * for a `tool` branch — the fired tool's `id@version`, or — for a `rejected`
 * branch (PR-3/A2) — the fail-closed `rejectionReason` the model re-prompts over.
 * Kept in its own module (the runs.ts module-per-concern precedent).
 *
 * SN-8: every id is server-derived; the SDK only *encodes* the bytes to hex.
 */

import type { ReactTurnSummary as PbReactTurnSummary } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One ReAct turn fact: hex ids + the frozen branch (+ the fired tool for a
 *  `tool` branch) + the run's durable budget caps + the journal seq (cursor). */
export class ReactTurn {
  constructor(
    readonly turn: number,
    readonly turnMoteId: string,
    readonly instanceId: string,
    readonly modelId: string,
    readonly branch: string,
    readonly toolId: string,
    readonly toolVersion: string,
    readonly maxTurns: number,
    readonly maxToolCalls: number,
    readonly seq: number,
    readonly rejectionReason: string = "",
  ) {}

  static fromProto(t: PbReactTurnSummary): ReactTurn {
    return new ReactTurn(
      t.turn,
      encode(t.turnMoteId),
      encode(t.instanceId),
      t.modelId,
      t.branch,
      t.toolId,
      t.toolVersion,
      t.maxTurns,
      t.maxToolCalls,
      Number(t.seq),
      t.rejectionReason,
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      turn: this.turn,
      turn_mote_id: this.turnMoteId,
      instance_id: this.instanceId,
      model_id: this.modelId,
      branch: this.branch,
      tool_id: this.toolId,
      tool_version: this.toolVersion,
      max_turns: this.maxTurns,
      max_tool_calls: this.maxToolCalls,
      seq: this.seq,
      rejection_reason: this.rejectionReason,
    };
  }
}

/** One page of {@link ReactTurn} (newest-first) plus the `hasMore` cursor flag. */
export interface ReactTurnPage {
  readonly turns: ReactTurn[];
  readonly hasMore: boolean;
}
