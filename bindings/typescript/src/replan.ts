/**
 * The re-plan-round view — one `ReplanRound` fact enumerated by `ListReplanRounds`
 * (PR-2c-2): the durable, queryable history of a run's model-driven re-plan loop
 * in `kx serve`. Each round carries its shaper Mote id, the resolved model, the
 * failed steps that triggered it, and whether the model escalated to a human (the
 * run quiesces). Kept in its own module (the runs.ts module-per-concern precedent).
 *
 * SN-8: ids are server-derived; the SDK only *encodes* the bytes to hex.
 */

import type { ReplanRoundSummary as PbReplanRoundSummary } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One re-plan round fact: the round index (0 = the initial-plan anchor), the
 *  shaper Mote id, the resolved model, the failed steps that triggered it, the
 *  escalation flag, and the journal seq (cursor). */
export class ReplanRound {
  constructor(
    readonly round: number,
    readonly shaperMoteId: string,
    readonly modelId: string,
    readonly failedStepIds: string[],
    readonly escalated: boolean,
    readonly seq: number,
  ) {}

  static fromProto(r: PbReplanRoundSummary): ReplanRound {
    return new ReplanRound(
      r.round,
      encode(r.shaperMoteId),
      r.modelId,
      r.failedStepIds.map((s) => encode(s)),
      r.escalated,
      Number(r.seq),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      round: this.round,
      shaper_mote_id: this.shaperMoteId,
      model_id: this.modelId,
      failed_step_ids: this.failedStepIds,
      escalated: this.escalated,
      seq: this.seq,
    };
  }
}

/** One page of {@link ReplanRound} (newest-first) plus the `hasMore` cursor flag. */
export interface ReplanRoundPage {
  readonly rounds: ReplanRound[];
  readonly hasMore: boolean;
}
