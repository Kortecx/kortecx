/**
 * The per-run quality readout (RC1/D172) — an EXPECTATION-FREE summary of a live run's
 * trajectory (terminal reached, turns / tool-calls spent, budget burn, rejection count),
 * surfaced by `ScoreRun`. The golden-suite GATE (task success / tool-call correctness /
 * groundedness vs a known expectation) runs OFFLINE via `kx eval run` — it never crosses
 * this wire. Kept in its own module (module-per-concern).
 */

import type { RunScore as PbRunScore } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** A live run's expectation-free quality summary (the `ScoreRun` readout). */
export class RunScore {
  constructor(
    readonly instanceId: string,
    readonly terminal: string,
    readonly reachedAnswer: boolean,
    readonly turnsUsed: number,
    readonly toolCallsUsed: number,
    readonly maxTurns: number,
    readonly maxToolCalls: number,
    readonly rejections: number,
    readonly turnBudgetUsedPerMille: number,
    readonly toolBudgetUsedPerMille: number,
  ) {}

  static fromProto(s: PbRunScore): RunScore {
    return new RunScore(
      encode(s.instanceId),
      s.terminal,
      s.reachedAnswer,
      s.turnsUsed,
      s.toolCallsUsed,
      s.maxTurns,
      s.maxToolCalls,
      s.rejections,
      s.turnBudgetUsedPerMille,
      s.toolBudgetUsedPerMille,
    );
  }

  toJSON() {
    return {
      instance_id: this.instanceId,
      terminal: this.terminal,
      reached_answer: this.reachedAnswer,
      turns_used: this.turnsUsed,
      tool_calls_used: this.toolCallsUsed,
      max_turns: this.maxTurns,
      max_tool_calls: this.maxToolCalls,
      rejections: this.rejections,
      turn_budget_used_per_mille: this.turnBudgetUsedPerMille,
      tool_budget_used_per_mille: this.toolBudgetUsedPerMille,
    };
  }
}
