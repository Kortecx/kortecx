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
import { getDefaultClient } from "./default-client.js";
import { KxUsage } from "./errors.js";
import type { SubmitWorkflowRequestSchema } from "./gen/kortecx/v1/gateway_pb.js";
import type { Result, Run } from "./run.js";
import { type LocalToolDef, isLocalTool, localToolNode } from "./tools.js";
import type { RegisterMcpServerInput } from "./toolscout.js";

/** A thing a builder method can fold in: a prompt (⇒ an agent step) or a `Frag`. */
export type FlowItem = string | Frag;

/** Options for {@link Flow.agent} (and the `then(string, …)` form). */
export interface AgentStepOptions {
  /** Tool grants — registered tool names and/or `localTool(...)` defs (V2b). */
  tools?: readonly (string | LocalToolDef)[] | Readonly<Record<string, string>>;
  model?: string;
  maxTurns?: number;
  maxToolCalls?: number;
  reasoning?: ReasoningMode;
}

/** AGENTIC-VISION: the step-config key a {@link Flow.image} ref binds into (the SAME key
 * the vision/react-vision recipes publish + the gateway executor / coordinator read). */
const IMAGE_REF_KEY = "image_ref";

function toFrag(item: FlowItem): Frag {
  // A bare string is an agent (MODEL) step with all-default config (the common case).
  return typeof item === "string" ? task.model("", item) : item;
}

/** A minimal client surface the Flow terminals need (avoids a node/web import cycle).
 * Intentionally loose (`Promise<unknown>`) so test doubles satisfy it; the terminals
 * narrow to `Run | Result` (the concrete {@link import("./client.js").KxClientBase}
 * return). */
export interface FlowClient {
  runChain(chain: Chain, opts?: { wait?: boolean; timeoutMs?: number }): Promise<unknown>;
  /** OPTIONAL — present on the real {@link import("./client.js").KxClient}. Used by
   * {@link Flow.withMcp} to register a connector before the flow submits. A test double
   * without it is fine UNLESS the flow uses `.withMcp(...)` (then `run()` throws). */
  registerMcpServer?(input: RegisterMcpServerInput): Promise<unknown>;
  /** OPTIONAL — present on the real {@link import("./client.js").KxClient}. Used by
   * {@link Flow.withMemory} to seed a durable memory before the flow submits (RC5a). */
  storeMemory?(content: string | Uint8Array, opts?: { kind?: number }): Promise<unknown>;
}

/** Resolve the client for a terminal: the explicit one, else the zero-config Node
 * default (installed by the `@kortecx/sdk` / `@kortecx/sdk/node` entry). The browser
 * & chains entrypoints install no default ⇒ a clear, actionable error. */
function resolveClient(explicit?: FlowClient): FlowClient {
  if (explicit !== undefined) return explicit;
  const c = getDefaultClient();
  if (c === undefined) {
    throw new KxUsage(
      "run() needs a client — pass { client }, or import from '@kortecx/sdk' (Node) for the " +
        "zero-config default (configurable via setDefaultClient / KX_ENDPOINT / ~/.kortecx/config.toml). " +
        "The browser (@kortecx/sdk/web) & chains entrypoints are explicit-client by design.",
    );
  }
  return c as FlowClient;
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
  /** Connectors to register (external MCP servers) BEFORE this flow submits — see
   * {@link withMcp}. Stored OFF the lowered graph so `toChain`/`build` stay
   * byte-identical (the golden digest holds). */
  private readonly mcp: RegisterMcpServerInput[] = [];
  /** RC5a: durable memory facts to REMEMBER BEFORE this flow submits — see
   * {@link withMemory}. Stored OFF the lowered graph so `toChain`/`build` stay
   * byte-identical (the golden digest holds). */
  private readonly memoryFacts: string[] = [];
  /** AGENTIC-VISION: an image ref pending for the NEXT agent step (set by {@link image},
   * consumed + cleared by {@link agent}). Per-step, so a multi-step flow can ground each
   * step with a different image. */
  private pendingImage: string | undefined;

  constructor(opts: { seed?: number } = {}) {
    this.seed = opts.seed ?? 0;
  }

  private append(frag: Frag): this {
    this.node = this.node === undefined ? frag : seq(this.node, frag);
    return this;
  }

  /** Append an agent (MODEL) step. `model` defaults to the served model (the client's
   * `defaultModel` fills a blank one at submit, SN-8); `tools` makes it a
   * deterministic-agentic step (PR-9b; execution is LIVE as of PR-9b-2).
   *
   * AGENTIC-VISION: a preceding {@link image} grounds this step — the served VLM reasons
   * over that image on every turn (the ref binds into the step's `config_subset[image_ref]`). */
  agent(prompt: string, opts: AgentStepOptions = {}): this {
    const image = this.pendingImage;
    this.pendingImage = undefined;
    const params: Record<string, string> = image !== undefined ? { [IMAGE_REF_KEY]: image } : {};
    return this.append(
      task.model(opts.model ?? "", prompt, params, {
        tools: opts.tools,
        maxTurns: opts.maxTurns,
        maxToolCalls: opts.maxToolCalls,
        reasoning: opts.reasoning,
      }),
    );
  }

  /** AGENTIC-VISION: attach an image to the NEXT agent step. `ref` is a 64-hex content
   * ref — upload the bytes once via `client.putContent(data).contentRef`, then ground one
   * or more agent steps with it. The served VLM reasons over the image on EVERY turn of
   * that step's loop (durably carried across the chain). Per-step: a later `.image()`
   * before another `.agent()` grounds that step with a different image. Lowers client-free
   * + deterministically (the golden tri-surface contract). */
  image(ref: string): this {
    this.pendingImage = ref;
    return this;
  }

  /** Append a PURE step. */
  step(params: Readonly<Record<string, string>> = {}): this {
    return this.append(task.pure(params));
  }

  /** Append a standalone TOOL step — fire ONE tool (PR-6b-2). `toolId` is either a
   * registered tool name OR a `localTool(...)` def (V2b — then the 2nd arg is its
   * args object, registered at the run terminal + fired deterministically). */
  tool(
    toolId: string,
    version?: string,
    args?: Readonly<Record<string, string | number | boolean>>,
  ): this;
  tool(toolId: LocalToolDef, args?: Readonly<Record<string, string | number | boolean>>): this;
  tool(
    toolId: string | LocalToolDef,
    versionOrArgs?: string | Readonly<Record<string, string | number | boolean>>,
    args: Readonly<Record<string, string | number | boolean>> = {},
  ): this {
    if (isLocalTool(toolId)) {
      const a = (versionOrArgs as Readonly<Record<string, string | number | boolean>>) ?? {};
      return this.append(localToolNode(toolId, a));
    }
    return this.append(task.tool(toolId, (versionOrArgs as string) ?? "1", args));
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

  /** Register an external MCP **connector** at run time, BEFORE this flow submits, so
   * its namespaced `<name>/<tool>` tools resolve for a downstream
   * `.agent({ tools: [...] })` / `.tool(...)` — connectors are thus reachable from the
   * SAME single chaining entry point as everything else:
   *
   * ```ts
   * await flow()
   *   .withMcp({ name: "fs", endpoint: "npx",
   *              args: ["-y", "@modelcontextprotocol/server-filesystem", "/data"] })
   *   .agent("list /data", { tools: ["fs/list_directory"] })
   *   .run({ client: kx });
   * ```
   *
   * Pure pre-submit sugar over {@link import("./client.js").KxClient.registerMcpServer}
   * (a connector = an external MCP server, see `kx-extension-sdk`). It does NOT change
   * the lowered workflow — {@link toChain} / {@link build} are byte-identical with or
   * without it, so the golden tri-surface digest holds; registration is an imperative
   * side effect, never a DAG node. Idempotent (server-derived id + upsert).
   * `credentialRef` names an env var / vault key — the secret VALUE never travels (D81). */
  withMcp(spec: RegisterMcpServerInput): this {
    this.mcp.push(spec);
    return this;
  }

  /** Register each {@link withMcp} connector (declaration order) before submit. */
  private async registerMcp(client: FlowClient): Promise<void> {
    if (this.mcp.length === 0) return;
    if (typeof client.registerMcpServer !== "function") {
      throw new KxUsage(
        "withMcp() needs a client that can register connectors — pass { client: new KxClient(...) }",
      );
    }
    for (const spec of this.mcp) await client.registerMcpServer(spec);
  }

  /**
   * Seed durable MEMORY facts (RC5a), BEFORE this flow submits, so a downstream
   * `.agent(...)` on a `kx/recipes/react-memory` chain can `recall` them — memory is
   * thus reachable from the SAME single chaining entry point as everything else:
   *
   *   flow()
   *     .withMemory(["the deadline is March 3rd", "the client prefers email"])
   *     .agent("when is my deadline?")
   *     .run()
   *
   * Pure pre-submit sugar over {@link import("./client.js").KxClient.storeMemory}
   * (content-addressed + idempotent). It does NOT change the lowered workflow —
   * `toChain`/`build` are byte-identical with or without it, so the golden digest
   * holds; the store is an imperative side effect, never a DAG node. Every memory is
   * scoped to the caller's own principal.
   */
  withMemory(facts: string | string[]): this {
    for (const fact of typeof facts === "string" ? [facts] : facts) this.memoryFacts.push(fact);
    return this;
  }

  /** Store each {@link withMemory} fact (declaration order) before submit. */
  private async registerMemory(client: FlowClient): Promise<void> {
    if (this.memoryFacts.length === 0) return;
    if (typeof client.storeMemory !== "function") {
      throw new KxUsage(
        "withMemory() needs a client that can store memories — pass { client: new KxClient(...) }",
      );
    }
    for (const fact of this.memoryFacts) await client.storeMemory(fact);
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

  /** Export this flow as a portable blueprint object (Batch B; via {@link Flow.toChain}). */
  toBlueprint(): ReturnType<Chain["toBlueprint"]> {
    return this.toChain().toBlueprint();
  }

  /** Write the portable blueprint JSON to `path` (Batch B; NODE-only via {@link Flow.toChain}). */
  export(path: string): Promise<void> {
    return this.toChain().export(path);
  }

  /** Submit and (by default) WAIT for the committed result, over `opts.client` or the
   * zero-config Node default client. `wait: false` returns a {@link Run} handle. */
  async run(
    opts: { wait?: boolean; timeoutMs?: number; client?: FlowClient } = {},
  ): Promise<Run | Result> {
    const client = resolveClient(opts.client);
    await this.registerMcp(client);
    await this.registerMemory(client);
    return client.runChain(this.toChain(), {
      wait: opts.wait ?? true,
      timeoutMs: opts.timeoutMs,
    }) as Promise<Run | Result>;
  }

  /** Submit without waiting — return a {@link Run} handle. Drive it with `.wait()` (the
   * first committed Mote), `.events()`, or `.tokens(mote)`. */
  async submit(opts: { client?: FlowClient } = {}): Promise<Run> {
    return (await this.run({ wait: false, client: opts.client })) as Run;
  }
}

/** Start a fluent chain: `flow().agent(...).then(...).run({ client })`. The headline
 * authoring surface — reads top-to-bottom, IDE-discoverable, defaults filled in. */
export function flow(opts: { seed?: number } = {}): Flow {
  return new Flow(opts);
}
