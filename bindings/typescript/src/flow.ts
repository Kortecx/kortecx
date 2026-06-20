/**
 * The fluent Flow builder — the headline authoring surface (Batch V2).
 *
 * ```ts
 * import { flow } from "@kortecx/sdk";
 *
 * const out = await flow()
 *   .agent("Research the topic", { tools: ["web-search"] })
 *   .then("Critique the findings")
 *   .run({ client: kx });
 * console.log(out.text);
 * ```
 *
 * A thin, discoverable veneer over the combinator API in `chains.ts`: every method
 * folds into the SAME `seq` / `par` fragment graph the string DSL lowers from, so a
 * `Flow` lowers BYTE-IDENTICALLY to the equivalent chain (a Flow is sugar, never a new
 * wire shape). SN-8: a Flow describes TOPOLOGY only — the server compiles + warrants.
 */

import type { MessageInitShape } from "@bufbuild/protobuf";
import {
  type Chain,
  ChainParseError,
  type Frag,
  type ReasoningMode,
  chainFrom,
  par,
  seq,
  task,
} from "./chains.js";
import type { SubmitWorkflowRequestSchema } from "./gen/kortecx/v1/gateway_pb.js";

/** A thing a builder method can fold in: a prompt (⇒ an agent step) or a `Frag`. */
export type FlowItem = string | Frag;

/** Options for {@link Flow.agent} (and the `then(string, …)` form). */
export interface AgentStepOptions {
  tools?: readonly string[] | Readonly<Record<string, string>>;
  model?: string;
  maxTurns?: number;
  maxToolCalls?: number;
  reasoning?: ReasoningMode;
}

function toFrag(item: FlowItem): Frag {
  // A bare string is an agent (MODEL) step with all-default config (the common case).
  return typeof item === "string" ? task.model("", item) : item;
}

/** A minimal client surface the Flow terminals need (avoids a node/web import cycle). */
export interface FlowClient {
  runChain(chain: Chain, opts?: { wait?: boolean; timeoutMs?: number }): Promise<unknown>;
}

/**
 * A fluent chain builder. Each method APPENDS to the graph and returns `this`;
 * terminate with {@link Flow.run} / {@link Flow.toChain}. The string DSL (`chain(...)`)
 * and the combinators (`seq`/`par`/`chainFrom`) remain available as power forms — all
 * lower identically.
 */
export class Flow {
  private node: Frag | undefined;
  private readonly seed: number;
  private readonly ctx: string[] = [];

  constructor(opts: { seed?: number } = {}) {
    this.seed = opts.seed ?? 0;
  }

  private append(frag: Frag): this {
    this.node = this.node === undefined ? frag : seq(this.node, frag);
    return this;
  }

  /** Append an agent (MODEL) step. `model` defaults to the served model (the client's
   * `defaultModel` fills a blank one at submit, SN-8); `tools` makes it a
   * deterministic-agentic step (PR-9b; execution lights up with PR-9b-2). */
  agent(prompt: string, opts: AgentStepOptions = {}): this {
    return this.append(
      task.model(
        opts.model ?? "",
        prompt,
        {},
        {
          tools: opts.tools,
          maxTurns: opts.maxTurns,
          maxToolCalls: opts.maxToolCalls,
          reasoning: opts.reasoning,
        },
      ),
    );
  }

  /** Append a PURE step. */
  step(params: Readonly<Record<string, string>> = {}): this {
    return this.append(task.pure(params));
  }

  /** Append a standalone TOOL step — fire ONE registered tool (PR-6b-2). */
  tool(
    toolId: string,
    version = "1",
    args: Readonly<Record<string, string | number | boolean>> = {},
  ): this {
    return this.append(task.tool(toolId, version, args));
  }

  /** Append `item` sequentially. A bare string is an agent step (with `opts`); a `Frag`
   * is appended as-is. Reads as the natural follow-on after {@link Flow.agent}. */
  // biome-ignore lint/suspicious/noThenProperty: Flow is a builder, not a thenable — terminate with `.run()` (the awaited Promise); a Flow is never awaited directly, and `.then(item)` mirrors the Python fluent API (cross-surface vocab).
  then(item: FlowItem, opts: AgentStepOptions = {}): this {
    if (typeof item === "string") return this.agent(item, opts);
    return this.append(item);
  }

  /** Append a PARALLEL fan of `items` as one merge node, sequential after the tail —
   * a fan-out when something precedes it, a fan-in when something follows. */
  parallel(...items: FlowItem[]): this {
    if (items.length === 0) throw new ChainParseError("parallel() needs at least one branch");
    return this.append(par(...items.map(toFrag)));
  }

  /** Attach context-bundle handles to the run (PR-7, chain-level grounding). */
  context(...handles: string[]): this {
    this.ctx.push(...handles);
    return this;
  }

  /** Lower this flow to a {@link Chain}. */
  toChain(): Chain {
    if (this.node === undefined) {
      throw new ChainParseError("empty flow — add a step (.agent / .step / .tool) first");
    }
    return chainFrom(this.node, { seed: this.seed, context: this.ctx });
  }

  /** Lower to a `SubmitWorkflow` request. */
  build(): MessageInitShape<typeof SubmitWorkflowRequestSchema> {
    return this.toChain().build();
  }

  /** The canonical pre-encoding lowering (the corpus-parity shape). */
  lower(): ReturnType<Chain["lower"]> {
    return this.toChain().lower();
  }

  /** Submit and (by default) WAIT for the committed result, over `opts.client`. */
  async run(opts: { wait?: boolean; timeoutMs?: number; client: FlowClient }): Promise<unknown> {
    return opts.client.runChain(this.toChain(), {
      wait: opts.wait ?? true,
      timeoutMs: opts.timeoutMs,
    });
  }
}

/** Start a fluent chain: `flow().agent(...).then(...).run({ client })`. The headline
 * authoring surface — reads top-to-bottom, IDE-discoverable, defaults filled in. */
export function flow(opts: { seed?: number } = {}): Flow {
  return new Flow(opts);
}
