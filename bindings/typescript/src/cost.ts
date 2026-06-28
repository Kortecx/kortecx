/**
 * The cost-spend guardrail readout (M11) — a run's DISPLAY-ONLY local spend estimate
 * over the durable turn/tool counters at operator-set micro-USD rates (`GetRunCost`).
 * A BUDGET GUARDRAIL readout, NOT Cloud per-expert billing (the D129/D156/GR19
 * boundary holds). Kept in its own module (module-per-concern).
 */

import type { GetRunCostResponse as PbRunCost } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** A run's local spend estimate (micro-USD), with the priced counters + rates. */
export class RunCost {
  constructor(
    readonly instanceId: string,
    readonly turns: number,
    readonly toolCalls: number,
    readonly estimatedMicroUsd: number,
    readonly ceilingMicroUsd: number,
    readonly perTurnMicroUsd: number,
    readonly perToolCallMicroUsd: number,
    readonly overCeiling: boolean,
  ) {}

  static fromProto(c: PbRunCost): RunCost {
    return new RunCost(
      encode(c.instanceId),
      Number(c.turns),
      Number(c.toolCalls),
      Number(c.estimatedMicroUsd),
      Number(c.ceilingMicroUsd),
      Number(c.perTurnMicroUsd),
      Number(c.perToolCallMicroUsd),
      c.overCeiling,
    );
  }

  toJSON() {
    return {
      instance_id: this.instanceId,
      turns: this.turns,
      tool_calls: this.toolCalls,
      estimated_micro_usd: this.estimatedMicroUsd,
      ceiling_micro_usd: this.ceilingMicroUsd,
      per_turn_micro_usd: this.perTurnMicroUsd,
      per_tool_call_micro_usd: this.perToolCallMicroUsd,
      over_ceiling: this.overCeiling,
    };
  }
}
