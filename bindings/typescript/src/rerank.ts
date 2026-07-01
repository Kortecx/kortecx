/**
 * The re-rank-turn view — one `ReRankRound` fact enumerated by `ListReRankTurns`
 * (RC4c-2): the durable, queryable history of a live listwise LLM re-rank loop in
 * `kx serve`. Each turn carries its run-salted re-rank Mote id, the resolved model,
 * the settled `outcome` (`pending` | `reranked` | `failed_closed`), the candidate
 * count, and — for a `reranked` outcome — the exact `permutation` (reordered source
 * indices) the runtime enforced. Kept in its own module (the react.ts / replan.ts
 * module-per-concern precedent).
 *
 * SN-8: every id is server-derived; the SDK only *encodes* the bytes to hex, and
 * the permutation is an exact reordering the runtime enforced, never a score.
 */

import type { ReRankTurnSummary as PbReRankTurnSummary } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One re-rank turn fact: the round index, the run-salted re-rank Mote id, the
 *  resolved model, the settled outcome, the candidate count, the enforced
 *  permutation (set iff `outcome === "reranked"`), and the journal seq (cursor). */
export class ReRankTurn {
  constructor(
    readonly round: number,
    readonly rerankMoteId: string,
    readonly instanceId: string,
    readonly modelId: string,
    readonly outcome: string,
    readonly candidateCount: number,
    /** The reordered source indices; set iff `outcome === "reranked"`. */
    readonly permutation: number[],
    readonly seq: number,
  ) {}

  static fromProto(t: PbReRankTurnSummary): ReRankTurn {
    return new ReRankTurn(
      t.round,
      encode(t.rerankMoteId),
      encode(t.instanceId),
      t.modelId,
      t.outcome,
      t.candidateCount,
      [...t.permutation],
      Number(t.seq),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      round: this.round,
      rerank_mote_id: this.rerankMoteId,
      instance_id: this.instanceId,
      model_id: this.modelId,
      outcome: this.outcome,
      candidate_count: this.candidateCount,
      permutation: this.permutation,
      seq: this.seq,
    };
  }
}

/** One page of {@link ReRankTurn} (newest-first) plus the `hasMore` cursor flag. */
export interface ReRankTurnPage {
  readonly turns: ReRankTurn[];
  readonly hasMore: boolean;
}
