/**
 * The agent-runner result — the one-object answer of `runAgent` (PR-9c-1).
 *
 * `runAgent({ goal }) → AgentResult`: the model's final answer PLUS the audited set
 * of tool actions it took (each a durable `ReactRound` `tool` fact, server-derived).
 * A thin, read-only projection over the steered `kx/recipes/react` chain — no new
 * wire surface, no proto change (assembled client-side from `ListReactTurns` +
 * `GetContent`). Platform-neutral (no Node imports): re-exported by both the node
 * and web entries via `common`. SN-8: every id + action is server-derived.
 */

import type { ReactTurn } from "./react.js";

/** One tool action the agent took — a settled ReAct `tool` turn. The `toolId` /
 *  `toolVersion` are the GRANTED tool's (SN-8), never the model's raw proposal. */
export class AuditedAction {
  constructor(
    readonly toolId: string,
    readonly toolVersion: string,
    readonly turn: number,
  ) {}

  static fromTurn(t: ReactTurn): AuditedAction {
    return new AuditedAction(t.toolId, t.toolVersion, t.turn);
  }

  /** A plain snake_case object (parity with the Python SDK + the CLI `--json`). */
  toJSON() {
    return { tool_id: this.toolId, tool_version: this.toolVersion, turn: this.turn };
  }
}

/** The terminal answer of an agent run + its audited action set + the durable,
 *  re-attachable run handle (the instance id). */
export class AgentResult {
  constructor(
    /** The final answer decoded UTF-8 (`null` if non-text / absent). */
    readonly answer: string | null,
    /** The raw committed answer bytes. */
    readonly answerBytes: Uint8Array | null,
    readonly actions: readonly AuditedAction[],
    /** hex instance id — the durable handle to re-attach to this run. */
    readonly runHandle: string,
    /** hex instance id (=== {@link runHandle}). */
    readonly instanceId: string,
  ) {}

  /** True iff the agent produced a committed answer. */
  get ok(): boolean {
    return this.answerBytes !== null;
  }

  /** A JSON-able view (the `kx agent run --json` shape; parity with the Python SDK). */
  toJSON(): Record<string, unknown> {
    const out: Record<string, unknown> = {
      instance_id: this.instanceId,
      run_handle: this.runHandle,
      actions: this.actions.map((a) => a.toJSON()),
    };
    if (this.answer !== null) out.answer = this.answer;
    return out;
  }

  /** Alias of {@link toJSON} (mirrors the Python `AgentResult.json()`). */
  json(): Record<string, unknown> {
    return this.toJSON();
  }
}

/** The audited action set = the chain's settled `tool` turns, in turn order. Pure
 *  client-side derivation over the durable `ListReactTurns` facts (no mutation of
 *  the input). */
export function assembleActions(turns: readonly ReactTurn[]): AuditedAction[] {
  return turns
    .filter((t) => t.branch === "tool")
    .sort((a, b) => a.turn - b.turn)
    .map((t) => AuditedAction.fromTurn(t));
}
