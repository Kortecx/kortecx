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
import { KxNotFound } from "./errors.js";
import { type FlowClient, flow } from "./flow.js";
import { KxToolError, type LocalToolDef, registerLocalTools } from "./tools.js";

/** The steered, dynamic-tool recipe (the model chooses tools turn by turn). */
export const REACT_RECIPE_HANDLE = "kx/recipes/react";
/** The steered lane that AUTO-GRANTS the live registered tool set (PR-6b-4) — the
 *  dynamic lane routes here when the agent carries tools (only react-auto fires a
 *  dialed/registered tool). Requires the serve to run with `KX_SERVE_AUTOGRANT=1`. */
export const REACT_AUTO_RECIPE_HANDLE = "kx/recipes/react-auto";

export interface AgentOptions {
  /** Tool grants — registered tool names and/or `localTool(...)` defs (V2b). */
  tools?: readonly (string | LocalToolDef)[] | Readonly<Record<string, string>>;
  model?: string;
  maxTurns?: number;
  maxToolCalls?: number;
  reasoning?: ReasoningMode;
  /** `false` (default) = the deterministic/frozen lane; `true` = the steered react recipe. */
  dynamic?: boolean;
}

/** The client surface {@link Agent.run} needs: `runChain` (frozen) + `invoke` (dynamic)
 *  + the MCP-gateway methods (V2b local-tool registration for the dynamic lane). */
export interface AgentClient extends FlowClient {
  invoke(
    handle: string,
    args: unknown,
    opts?: { wait?: boolean; timeoutMs?: number },
  ): Promise<unknown>;
  registerMcpServer(input: {
    name: string;
    transport: string;
    endpoint: string;
    args: string[];
  }): Promise<unknown>;
  discoverServerTools(name: string): Promise<{ tools: ReadonlyArray<{ toolName: string }> }>;
}

function hasTools(tools: AgentOptions["tools"]): boolean {
  if (tools === undefined) {
    return false;
  }
  return Array.isArray(tools) ? tools.length > 0 : Object.keys(tools).length > 0;
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

  /**
   * Run `task`.
   *
   * - **frozen lane (default)** ⇒ a single agent step. A tool-bearing frozen agent runs
   *   a deterministic-agentic loop that **lands in PR-9b-2** (refused at submit today),
   *   so a clear pre-flight hint is thrown; use `dynamic: true` or `flow().tool(fn, args)`.
   * - `dynamic: true` ⇒ the steered react lane. With tools it routes to
   *   `kx/recipes/react-auto` (the only lane that fires registered/dialed tools; needs
   *   `KX_SERVE_AUTOGRANT=1`); without tools, plain `kx/recipes/react`.
   */
  async run(
    task: string,
    opts: { wait?: boolean; timeoutMs?: number; client: AgentClient },
  ): Promise<unknown> {
    const tools = this.opts.tools;
    if (this.opts.dynamic) {
      // The react / react-auto recipes REQUIRE the bounded-loop budget (the
      // `react_contract` slots; the UI's planReactArgs mirrors this) — default to
      // the recipe's anchored 8 / 6 when the agent didn't set them.
      const args = {
        instruction: this.prompt(task),
        max_turns: this.opts.maxTurns ?? 8,
        max_tool_calls: this.opts.maxToolCalls ?? 6,
      };
      const runOpts = { wait: opts.wait ?? true, timeoutMs: opts.timeoutMs };
      if (!hasTools(tools)) {
        return opts.client.invoke(REACT_RECIPE_HANDLE, args, runOpts);
      }
      await registerLocalTools(opts.client, tools);
      try {
        return await opts.client.invoke(REACT_AUTO_RECIPE_HANDLE, args, runOpts);
      } catch (e) {
        if (e instanceof KxNotFound) {
          throw new KxToolError(
            "the dynamic tool lane needs the 'kx/recipes/react-auto' recipe — serve with " +
              "KX_SERVE_AUTOGRANT=1 to enable it (it auto-grants the registered tool set to the loop)",
          );
        }
        throw e;
      }
    }
    if (hasTools(tools)) {
      throw new KxToolError(
        "a frozen Agent with a tool set runs a deterministic-agentic loop that lands in " +
          "PR-9b-2 and is refused at submit today; use { dynamic: true } for the steered react " +
          "lane, or flow().tool(fn, args) to fire one tool deterministically",
      );
    }
    return this.asFlow(task).run({
      wait: opts.wait,
      timeoutMs: opts.timeoutMs,
      client: opts.client,
    });
  }
}
