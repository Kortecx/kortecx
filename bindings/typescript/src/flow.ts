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
import { type AppBuilder, app } from "./app.js";
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

/** The default synthesizer prompt for {@link Flow.swarm} / {@link Flow.team} — a MODEL
 * gather reads every participant's committed output (its Data-edge parents, F-7). */
const DEFAULT_SWARM_GATHER =
  "You are the lead. Synthesize the parallel agents' results above into one coherent, " +
  "complete answer. Reconcile disagreements, keep what is well-supported, and drop redundancy.";
const DEFAULT_FAN_GATHER = "Combine the parallel results above into one coherent answer.";
const DEFAULT_REDUCE = "Reduce the mapper results above into one consolidated result.";

/** A swarm/team participant: a prompt, a `[prompt, tools]` tuple, an Agent /
 * persona (duck-typed), a {@link Flow} (already task-bound), or a Frag. */
export type SwarmParticipant =
  | string
  | readonly [string, AgentStepOptions["tools"]?]
  | Frag
  | Flow
  | AgentLike;

/** The minimal Agent shape a participant can be (duck-typed to avoid the flow↔agent
 * import cycle): public `instructions` + an `opts` config bag. */
interface AgentLike {
  instructions: string;
  opts: AgentStepOptions & { dynamic?: boolean; persona?: string };
}

function isAgentLike(x: unknown): x is AgentLike {
  return (
    typeof x === "object" &&
    x !== null &&
    typeof (x as AgentLike).instructions === "string" &&
    typeof (x as AgentLike).opts === "object"
  );
}

/** Options for {@link Flow.swarm} / the top-level {@link swarm}. */
export interface SwarmOptions {
  /** The shared task each participant works on (appended to its role/prompt). */
  goal?: string;
  /** The gather sink: a synthesis prompt (a MODEL step) or an explicit Frag. Default = a
   * MODEL synthesizer with a sensible prompt. */
  gather?: string | Frag;
  /** `true` (default) = a MODEL synthesizer gather; `false` = a PURE deterministic fold. */
  synthesize?: boolean;
}
/** Options for {@link Flow.team} (a swarm that always synthesizes). */
export type TeamOptions = Omit<SwarmOptions, "synthesize">;
/** Options for {@link Flow.fanOutGather}. */
export interface FanOptions {
  gather?: string | Frag;
  synthesize?: boolean;
}
/** Options for {@link Flow.mapReduce}. */
export interface ReduceOptions {
  reduce?: string | Frag;
  synthesize?: boolean;
}

function joinGoal(text: string, goal: string): string {
  return goal ? `${text}\n\n${goal}`.trim() : text;
}

/** Build the fan-in sink: a MODEL synthesizer (`gather` a prompt string, or the
 * `defaultPrompt` when `synthesize`), an explicit sink Frag, or a PURE fold. */
function sinkFrag(
  gather: string | Frag | undefined,
  synthesize: boolean,
  defaultPrompt: string,
): Frag {
  if (typeof gather === "string") return task.model("", gather);
  if (gather !== undefined) return gather;
  if (synthesize) return task.model("", defaultPrompt);
  return task.pure({});
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

  // -- orchestration (parallel agentic patterns; pure client composition) --

  /** Resolve ONE swarm participant to an agentic-leaf Frag (mirrors Python
   * `_participant_to_node`; Agents duck-typed to avoid the flow↔agent cycle). */
  private resolveParticipant(item: SwarmParticipant, goal: string): Frag {
    if (typeof item === "string") return task.model("", joinGoal(item, goal));
    if (Array.isArray(item)) {
      const [prompt, tools] = item as readonly [string, AgentStepOptions["tools"]?];
      return task.model("", joinGoal(String(prompt), goal), {}, { tools });
    }
    if (item instanceof Flow) {
      if (item.node === undefined) throw new ChainParseError("empty flow participant");
      return item.node;
    }
    if (isAgentLike(item)) {
      const base = item.instructions;
      // Forward the Agent's pinned model as model_id (Py↔TS parity); instructions+goal
      // are the PROMPT.
      return task.model(
        item.opts.model ?? "",
        base ? joinGoal(base, goal) : goal,
        {},
        {
          tools: item.opts.tools,
          maxTurns: item.opts.maxTurns,
          maxToolCalls: item.opts.maxToolCalls,
          reasoning: item.opts.reasoning,
        },
      );
    }
    return item as Frag;
  }

  /** Fan out to N parallel agents, then gather (a **swarm**). Each participant is a
   * prompt, a `[prompt, tools]` tuple, an Agent / persona, or a Flow; they run
   * CONCURRENTLY as independent deterministic-agentic steps (each its own crash-safe
   * salt-2 chain), then a gather step merges their committed outputs:
   *
   * ```ts
   * await kx.flow()
   *   .swarm([kx.persona("researcher"), kx.persona("critic"), kx.persona("writer")],
   *          { goal: "Write a briefing on durable execution" })
   *   .run();
   * ```
   *
   * `opts.goal` is the shared task each participant works on. By default
   * (`synthesize: true`) the gather is a MODEL step that reads every participant's output
   * (its Data-edge parents, F-7) and writes one answer; `opts.gather: "<prompt>"` steers
   * it, a Frag gives a custom sink, and `synthesize: false` folds deterministically
   * (PURE). Pure client composition — byte-identical to the equivalent `[a & b] > g`
   * chain; no new step kind. */
  swarm(participants: SwarmParticipant[], opts: SwarmOptions = {}): this {
    if (participants.length === 0) throw new ChainParseError("swarm() needs at least one agent");
    const goal = opts.goal ?? "";
    this.parallel(...participants.map((a) => this.resolveParticipant(a, goal)));
    return this.then(sinkFrag(opts.gather, opts.synthesize ?? true, DEFAULT_SWARM_GATHER));
  }

  /** A **team**: the same topology as {@link Flow.swarm} with a lead that synthesizes.
   * `team(a, { goal })` ≡ `swarm(a, { goal, synthesize: true })`. */
  team(participants: SwarmParticipant[], opts: TeamOptions = {}): this {
    return this.swarm(participants, { ...opts, synthesize: true });
  }

  /** Fan out to N parallel `branches` (each a prompt / Frag), then gather —
   * sample-N-ways-and-combine. `opts.gather` steers the sink; `synthesize: false` folds
   * deterministically (PURE). */
  fanOutGather(branches: FlowItem[], opts: FanOptions = {}): this {
    if (branches.length === 0)
      throw new ChainParseError("fanOutGather() needs at least one branch");
    this.parallel(...branches);
    return this.then(sinkFrag(opts.gather, opts.synthesize ?? true, DEFAULT_FAN_GATHER));
  }

  /** Map N `mappers` in parallel, then reduce. `opts.reduce` steers the reducer;
   * `synthesize: false` reduces deterministically (PURE). */
  mapReduce(mappers: FlowItem[], opts: ReduceOptions = {}): this {
    if (mappers.length === 0) throw new ChainParseError("mapReduce() needs at least one mapper");
    this.parallel(...mappers);
    return this.then(sinkFrag(opts.reduce, opts.synthesize ?? true, DEFAULT_REDUCE));
  }

  /** Promote this Flow to a durable, named {@link import("./app.js").AppBuilder} — the
   * EXPLICIT boundary (D177) from ad-hoc authoring to a shareable App that runs via
   * `RunApp` (server-resolved connections + secret_scope + skills). Chain the App rails
   * on the result:
   *
   * ```ts
   * await kx.flow().agent("Draft and send a reply", { tools: ["kx-connector-gmail/send"] })
   *   .asApp("mailer").withGmail().secrets(["KX_GMAIL_CREDENTIAL"])
   *   .run({ to: "x@y.com" });
   * ```
   *
   * Naming is deliberate: connections / skills / secret scope ride the App envelope
   * (not the byte-identical lowered graph), so a bare `Flow.run()` has no place for them.
   * Any `withMcp` / `withMemory` side-channels carry over as pre-run registrations. */
  asApp(name: string, opts: { version?: string } = {}): AppBuilder {
    const built = app(name, opts).blueprint(this);
    built.carryFlowSideChannels(this.mcp, this.memoryFacts);
    return built;
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

// -- top-level orchestration factories (a swarm is usually the whole flow) --

/** `swarm(participants, opts)` — N parallel agents → gather, as a whole flow.
 * Sugar for `flow(seed).swarm(...)`; see {@link Flow.swarm}. */
export function swarm(
  participants: SwarmParticipant[],
  opts: SwarmOptions & { seed?: number } = {},
): Flow {
  return flow({ seed: opts.seed }).swarm(participants, opts);
}

/** `team(participants, opts)` — a swarm with a lead that synthesizes; see {@link Flow.team}. */
export function team(
  participants: SwarmParticipant[],
  opts: TeamOptions & { seed?: number } = {},
): Flow {
  return flow({ seed: opts.seed }).team(participants, opts);
}

/** `fanOutGather(branches, opts)` — sample N ways, combine; see {@link Flow.fanOutGather}. */
export function fanOutGather(
  branches: FlowItem[],
  opts: FanOptions & { seed?: number } = {},
): Flow {
  return flow({ seed: opts.seed }).fanOutGather(branches, opts);
}

/** `mapReduce(mappers, opts)` — map N mappers in parallel, then reduce; see
 * {@link Flow.mapReduce}. */
export function mapReduce(mappers: FlowItem[], opts: ReduceOptions & { seed?: number } = {}): Flow {
  return flow({ seed: opts.seed }).mapReduce(mappers, opts);
}
