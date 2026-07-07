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
 * recipe. The frozen tool-EXECUTION is LIVE — the `Agent({ tools: [fn] })` one-liner over
 * local `localTool(...)` defs now fires (`run` registers each as a stdio MCP tool and
 * grants its namespaced `<server>/<name>` to the step; a model's bare/leaf call resolves
 * to that grant). No `KX_SERVE_AUTOGRANT` needed. SN-8: intent only.
 */

import type { ReasoningMode } from "./chains.js";
import type { ImageInput } from "./client.js";
import { getDefaultClient } from "./default-client.js";
import { KxNotFound, KxUsage } from "./errors.js";
import { type FlowClient, flow } from "./flow.js";
import { PERSONAS } from "./personas.js";
import type { Result, Run } from "./run.js";
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
  /** A curated persona name (from {@link import("./personas.js").PERSONAS}) whose
   * role instructions become this agent's base instructions. Any explicit `instructions`
   * layer on top. Throws for an unknown name. */
  persona?: string;
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
  /** AGENTIC-VISION: resolve `image` + bind `kx/recipes/react-vision` (form-gated). */
  bindReactVision(
    args: Record<string, unknown>,
    image: ImageInput,
  ): Promise<{ handle: string; args: Record<string, unknown> }>;
}

/** Resolve the client for an Agent terminal: the explicit one, else the zero-config
 * Node default. The browser & chains entrypoints install no default ⇒ a clear error. */
function resolveAgentClient(explicit?: AgentClient): AgentClient {
  if (explicit !== undefined) return explicit;
  const c = getDefaultClient();
  if (c === undefined) {
    throw new KxUsage(
      "agent.run() needs a client — pass { client }, or import from '@kortecx/sdk' (Node) for " +
        "the zero-config default (set via setDefaultClient / KX_ENDPOINT / ~/.kortecx/config.toml). " +
        "The browser & chains entrypoints are explicit-client by design.",
    );
  }
  return c as AgentClient;
}

function hasTools(tools: AgentOptions["tools"]): boolean {
  if (tools === undefined) {
    return false;
  }
  return Array.isArray(tools) ? tools.length > 0 : Object.keys(tools).length > 0;
}

/** A reusable agent: instructions + an optional tool set + model/loop config. */
export class Agent {
  readonly instructions: string;

  constructor(
    instructions = "",
    readonly opts: AgentOptions = {},
  ) {
    let resolved = instructions;
    if (opts.persona !== undefined) {
      const base = PERSONAS[opts.persona];
      if (base === undefined) {
        throw new KxUsage(`unknown persona ${JSON.stringify(opts.persona)}`);
      }
      // A curated role; explicit `instructions` (if any) layer on top of it.
      resolved = instructions ? `${base}\n\n${instructions}`.trim() : base;
    }
    this.instructions = resolved;
  }

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

  /** Bind this agent to `task` → a {@link import("./flow.js").Flow} (a thin alias of
   * {@link Agent.asFlow}; reads as `researcher.on("topic A")`). */
  on(task: string) {
    return this.asFlow(task);
  }

  /**
   * Run `task`.
   *
   * - **frozen lane (default)** ⇒ a single agent step. The tool-bearing frozen loop
   *   EXECUTION is LIVE — the `Agent({ tools: [fn] })` one-liner over LOCAL functions
   *   fires (`run` resolves each `localTool(...)` to its namespaced grant on the step),
   *   as do EXPLICIT refs (`flow().model(prompt, { tools: ["mcp-echo"] })` / the
   *   `model@tool` chain DSL / a UI builder model step). No `KX_SERVE_AUTOGRANT` needed.
   * - `dynamic: true` ⇒ the steered react lane. With tools it routes to
   *   `kx/recipes/react-auto` (the only lane that fires registered/dialed tools; needs
   *   `KX_SERVE_AUTOGRANT=1`); without tools, plain `kx/recipes/react`.
   */
  async run(
    task: string,
    opts: { image?: ImageInput; wait?: boolean; timeoutMs?: number; client?: AgentClient } = {},
  ): Promise<Run | Result> {
    const client = resolveAgentClient(opts.client);
    const tools = this.opts.tools;
    if (opts.image) {
      // AGENTIC-VISION: an attached image routes to the image-grounded ReAct loop
      // (`kx/recipes/react-vision`, form-gated) so the served VLM reasons over the image
      // on every turn. The bounded-loop budget mirrors the dynamic lane; local custom
      // tools + an image is a future combo. Fail-closed when no vision model (GR15).
      const baseArgs = {
        instruction: this.prompt(task),
        max_turns: this.opts.maxTurns ?? 8,
        max_tool_calls: this.opts.maxToolCalls ?? 6,
      };
      const { handle, args } = await client.bindReactVision(baseArgs, opts.image);
      return client.invoke(handle, args, {
        wait: opts.wait ?? true,
        timeoutMs: opts.timeoutMs,
      }) as Promise<Run | Result>;
    }
    if (this.opts.dynamic) {
      // The react / react-auto recipes REQUIRE the bounded-loop budget (the
      // `react_contract` slots; the UI's planReactArgs mirrors this) — default to
      // the recipe's anchored 8 / 20 when the agent didn't set them (the tool-call cap
      // rose 6 → 20 at T-MULTI-ELEMENT-TOOLCALLS — a turn can fire N tools at once).
      const args = {
        instruction: this.prompt(task),
        max_turns: this.opts.maxTurns ?? 8,
        max_tool_calls: this.opts.maxToolCalls ?? 20,
      };
      const runOpts = { wait: opts.wait ?? true, timeoutMs: opts.timeoutMs };
      if (!hasTools(tools)) {
        return client.invoke(REACT_RECIPE_HANDLE, args, runOpts) as Promise<Run | Result>;
      }
      await registerLocalTools(client, tools);
      try {
        return (await client.invoke(REACT_AUTO_RECIPE_HANDLE, args, runOpts)) as Run | Result;
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
    // Frozen lane (with OR without tools): a single agent step whose tool-grant SET is
    // part of the step's identity (replayable). `asFlow` → `runChain` resolves any local
    // `localTool(...)` defs to their namespaced `<server>/<name>` and writes them into the
    // step's toolContract; the served model fires them in a bounded reason→tool→observe
    // loop (a model's bare/leaf name resolves to the grant — the BUG-32 fix). No
    // `KX_SERVE_AUTOGRANT` needed: the step grants its OWN exact tools (SN-8 — the server
    // still compiles + warrants every step).
    return this.asFlow(task).run({
      wait: opts.wait,
      timeoutMs: opts.timeoutMs,
      client,
    });
  }

  /**
   * Start `task` WITHOUT waiting and return a {@link Run}. Consume the live tail with
   * `.events()` (run-level deltas) or `.tokens(mote)` (one model mote's ADVISORY token
   * chunks). The `dynamic: true` lane returns a Run over the react recipe (its terminal
   * supports `.tokens()` with no arg); the frozen lane returns a workflow Run (pass a
   * `moteId` to `.tokens()`). The committed result stays the authority — finish with
   * `run.wait()`.
   */
  async stream(task: string, opts: { client?: AgentClient } = {}): Promise<Run | Result> {
    return this.run(task, { wait: false, client: opts.client });
  }
}
