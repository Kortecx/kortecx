/**
 * A first-class Agent (Batch V2) — the TS mirror of the Python `kx.Agent`.
 *
 * ```ts
 * const analyst = new Agent("You are a research analyst.", { tools: ["web-search"] });
 * const out = await analyst.run("Summarize the README", { client: kx });
 * console.log(out.text);
 * ```
 *
 * Default lane = deterministic/frozen (a single agent step — replayable, the tool set
 * is part of identity); `dynamic: true` routes to the steered `kx/recipes/react`
 * recipe. The frozen tool-EXECUTION lights up with PR-9b-2. SN-8: intent only.
 */

import type { ReasoningMode } from "./chains.js";
import { type FlowClient, flow } from "./flow.js";

/** The steered, dynamic-tool recipe (the model chooses tools turn by turn). */
export const REACT_RECIPE_HANDLE = "kx/recipes/react";

export interface AgentOptions {
  tools?: readonly string[] | Readonly<Record<string, string>>;
  model?: string;
  maxTurns?: number;
  maxToolCalls?: number;
  reasoning?: ReasoningMode;
  /** `false` (default) = the deterministic/frozen lane; `true` = the steered react recipe. */
  dynamic?: boolean;
}

/** The client surface {@link Agent.run} needs: `runChain` (frozen) + `invoke` (dynamic). */
export interface AgentClient extends FlowClient {
  invoke(
    handle: string,
    args: unknown,
    opts?: { wait?: boolean; timeoutMs?: number },
  ): Promise<unknown>;
}

/** A reusable agent: instructions + an optional tool set + model/loop config. */
export class Agent {
  constructor(
    readonly instructions = "",
    readonly opts: AgentOptions = {},
  ) {}

  private prompt(task: string): string {
    return this.instructions ? `${this.instructions}\n\n${task}`.trim() : task;
  }

  /** The FROZEN-lane {@link import("./flow.js").Flow} for `task` — one agent step. */
  asFlow(task: string) {
    return flow().agent(this.prompt(task), {
      tools: this.opts.tools,
      model: this.opts.model,
      maxTurns: this.opts.maxTurns,
      maxToolCalls: this.opts.maxToolCalls,
      reasoning: this.opts.reasoning,
    });
  }

  /** Run `task`. Frozen lane (default) ⇒ a single agent step; `dynamic` ⇒ the steered
   * `kx/recipes/react` recipe. Waits for the committed result unless `wait: false`. */
  async run(
    task: string,
    opts: { wait?: boolean; timeoutMs?: number; client: AgentClient },
  ): Promise<unknown> {
    if (this.opts.dynamic) {
      return opts.client.invoke(
        REACT_RECIPE_HANDLE,
        { instruction: this.prompt(task) },
        { wait: opts.wait ?? true, timeoutMs: opts.timeoutMs },
      );
    }
    return this.asFlow(task).run({
      wait: opts.wait,
      timeoutMs: opts.timeoutMs,
      client: opts.client,
    });
  }
}
