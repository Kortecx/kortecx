/**
 * The Chains DSL — compose a vetted palette of task handles (`pure` / `model` /
 * `tool`) into a Tier-1 DAG via a string expression OR a combinator API, then
 * lower to the EXISTING {@link BlueprintBuilder} for `SubmitWorkflow`. A MODEL
 * handle may tag tools with the `@` grammar (`plan@web-search@fs-list`, PR-9b /
 * D161.1) to become a **deterministic-agentic step** (a bounded reason→tool→observe
 * loop over a server-vetted tool SET). Kept in
 * its own module (the Rust core's module-per-concern discipline); it builds
 * `StepInput`/`EdgeInput` lists and feeds the builder — it NEVER reassembles a
 * `SubmitWorkflowRequest` itself.
 *
 * The grammar + canonical lowering are the cross-surface contract in
 * `tests/golden/chains/SPEC.md`, pinned by `corpus.json`: the Python, TypeScript,
 * and Rust (CLI) surfaces all parse + lower to byte-identical `(steps, edges)`.
 *
 * SN-8: the DSL describes topology ONLY. The server still compiles + warrants
 * every step (derives identity, builds every warrant from the party's grants);
 * a chain only changes what is PROPOSED, never the identity a step gets.
 */

import type { MessageInitShape } from "@bufbuild/protobuf";
import { BlueprintBuilder, type EdgeInput, type StepInput, type StepKind } from "./blueprints.js";
import type { SubmitWorkflowRequestSchema } from "./gen/kortecx/v1/gateway_pb.js";
// V2b: a `@kx.localTool` function-as-tool def — type-only here (the runtime dep is
// one-way tools.ts → chains.ts; this never creates an import cycle).
import type { LocalToolDef } from "./tools.js";

/** Error class for a malformed expression / empty group (the `parse` corpus class). */
export class ChainParseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ChainParseError";
  }
}

/** Error class for a parsed handle absent from the `tasks` map (`unknown_handle`). */
export class ChainUnknownHandleError extends Error {
  constructor(readonly handle: string) {
    super(`unknown task handle '${handle}'`);
    this.name = "ChainUnknownHandleError";
  }
}

/** Error class for a cycle / self-loop in the lowered DAG (the `cycle` class). */
export class ChainCycleError extends Error {
  constructor(message = "chain expression forms a cycle") {
    super(message);
    this.name = "ChainCycleError";
  }
}

/**
 * Error class for `@` tool grants tagged onto a non-model handle (PR-9b — the
 * `agentic_non_model` corpus class). The deterministic-agentic step requires a
 * MODEL handle.
 */
export class ChainAgenticError extends Error {
  constructor(
    readonly handle: string,
    readonly kind: string,
  ) {
    super(
      `\`@\` tool grants on a non-model step '${handle}' (kind '${kind}'); \`@tool\` tags require a model step`,
    );
    this.name = "ChainAgenticError";
  }
}

/**
 * A task node — a leaf fragment in the DSL. `pure` carries optional params; `model`
 * carries a model id + prompt (+ optional params). Identity is by OBJECT: reusing
 * the same `Task` instance reuses the same node (builds DAGs via fan-in/fan-out).
 * Construct via the {@link task} factories; do not mutate after composing.
 */
export class Task {
  constructor(
    readonly kind: "pure" | "model" | "tool",
    readonly modelId: string,
    readonly prompt: string,
    /** Pre-encoding lowering form — `params` values are strings (UTF-8-encoded at build). */
    readonly params: Readonly<Record<string, string>>,
    /**
     * TOOL: the single `{ tool_id: tool_version }` the step fires (PR-6b-2). MODEL:
     * the agentic grant SET (PR-9b, D161.1) — a non-empty contract makes a model
     * step a deterministic-agentic step (a bounded reason→tool→observe loop).
     */
    readonly toolContract: Readonly<Record<string, string>> = {},
    /** Agentic MODEL step only (PR-9b): the bounded-loop budget (default 8 / 6). */
    readonly maxTurns?: number,
    readonly maxToolCalls?: number,
    /**
     * V2b (local tools): `localTool(...)` defs referenced by this step. Off the wire
     * + the lowering — the SDK registers each as a stdio MCP server at the run
     * terminal and folds the server-derived name into `toolContract` ({@link Chain.build}).
     */
    readonly localTools: readonly LocalToolDef[] = [],
    /**
     * APP ONLY: the per-node capability BINDINGS — catalog skill names, connection
     * descriptors, dataset names this step uses. They name entries in an App envelope's
     * `references`, so they are meaningful ONLY when this chain becomes an App (`app(...)`);
     * on the plain workflow path {@link Chain.build} / {@link Chain.fromBlueprint} have no
     * `references` to resolve them against and REFUSE a non-empty one. Carried on the step,
     * not the chain, because a fan-out binds different capabilities to different nodes.
     */
    readonly appSkills: readonly string[] = [],
    readonly appConnections: readonly string[] = [],
    readonly appDatasets: readonly string[] = [],
    /**
     * APP ONLY — the App HANDLES this step calls. The odd one out: the three above give
     * this step more to work with, while this one lowers another App's whole blueprint into
     * the run and feeds its result to this step.
     */
    readonly appApps: readonly string[] = [],
  ) {}

  /** True when this step carries an App-envelope capability binding (skills / connections /
   *  datasets) — meaningful only on the App path; refused on the workflow path. */
  hasAppBindings(): boolean {
    return (
      this.appSkills.length > 0 ||
      this.appConnections.length > 0 ||
      this.appDatasets.length > 0 ||
      this.appApps.length > 0
    );
  }

  /** The {@link StepInput} this task lowers to (the builder encodes params). */
  toStepInput(): StepInput {
    return {
      kind: this.kind,
      modelId: this.modelId,
      prompt: this.prompt,
      toolContract: { ...this.toolContract },
      params: effectiveParams(this),
    };
  }
}

/**
 * PR-9b (D161.1): the canonical `params` keys a deterministic-agentic MODEL step's
 * bounded-loop budget rides under (decimal-string ⇒ canonical-JSON `u32`, the form
 * the coordinator's `react_seed_params` reads). MUST equal the Rust
 * `kx_mote::REACT_MAX_TURNS_KEY` / `REACT_MAX_TOOL_CALLS_KEY` + the Python keys.
 */
export const REACT_MAX_TURNS_KEY = "max_turns";
export const REACT_MAX_TOOL_CALLS_KEY = "max_tool_calls";

/**
 * Batch A: the canonical `params` key for the opt-in reasoning mode. The SERVER reads
 * `config_subset["reasoning"]` (`kx-gateway` `REASONING_KEY`); `full` / `minimal` /
 * `off` / `strip` steer the model's native think mode, any other / absent ⇒ the
 * model's own behavior. Setting it ⇒ a new MoteId; absent ⇒ byte-identical.
 */
export const REASONING_KEY = "reasoning";

const REASONING_MODES = ["full", "minimal", "off", "strip"] as const;
/** The opt-in reasoning modes the `reasoning` kwarg accepts. */
export type ReasoningMode = (typeof REASONING_MODES)[number];

function validateReasoning(reasoning: string): string {
  if (!(REASONING_MODES as readonly string[]).includes(reasoning)) {
    throw new ChainParseError(
      `reasoning must be one of ${REASONING_MODES.join(", ")}, got '${reasoning}'`,
    );
  }
  return reasoning;
}

/**
 * The step's params with the agentic-loop budget injected for a MODEL step carrying
 * a non-empty `toolContract` (PR-9b — mirrors the Rust `to_request` + the Python
 * `_effective_params`). Pure; absent budget ⇒ the coordinator default.
 */
function effectiveParams(t: Task): Record<string, string> {
  const params: Record<string, string> = { ...t.params };
  if (t.kind === "model" && Object.keys(t.toolContract).length > 0) {
    if (t.maxTurns !== undefined) {
      params[REACT_MAX_TURNS_KEY] = String(t.maxTurns);
    }
    if (t.maxToolCalls !== undefined) {
      params[REACT_MAX_TOOL_CALLS_KEY] = String(t.maxToolCalls);
    }
  }
  return params;
}

/**
 * Normalize an agentic-step tool grant set to a `{ name: version }` contract: an
 * array of names → version `"1"` (the `@tool` grammar default), a record → verbatim.
 */
function grantsToContract(
  tools?: readonly string[] | Readonly<Record<string, string>>,
): Record<string, string> {
  if (tools === undefined) {
    return {};
  }
  if (Array.isArray(tools)) {
    const contract: Record<string, string> = {};
    for (const name of tools) {
      if (!(name in contract)) {
        contract[name] = "1";
      }
    }
    return contract;
  }
  return { ...(tools as Record<string, string>) };
}

/** V2b: anything accepted in `tools: [...]` — a registered tool NAME or a `localTool(...)`. */
export type ToolRef = string | LocalToolDef;

/** Split a `tools=` value into (string/record grants, local-tool defs). A non-string,
 * non-`localTool` array item is a fail-closed authoring error. */
function splitTools(tools?: readonly ToolRef[] | Readonly<Record<string, string>>): {
  strings?: readonly string[] | Readonly<Record<string, string>>;
  locals: LocalToolDef[];
} {
  if (tools === undefined || !Array.isArray(tools)) {
    return { strings: tools as Readonly<Record<string, string>> | undefined, locals: [] };
  }
  const strings: string[] = [];
  const locals: LocalToolDef[] = [];
  for (const t of tools) {
    if (t && typeof t === "object" && (t as { __kxLocalTool?: boolean }).__kxLocalTool === true) {
      locals.push(t as LocalToolDef);
    } else if (typeof t === "string") {
      strings.push(t);
    } else {
      throw new ChainParseError("a tool must be a registered name (string) or a localTool(...)");
    }
  }
  return { strings: strings.length > 0 ? strings : undefined, locals };
}

/**
 * PR-6b-2: the single canonical `config_subset` key a `tool()` step's authored
 * args ride under. MUST equal the Rust `kx_mote::TOOL_ARGS_KEY` + the Python
 * `TOOL_ARGS_KEY` (the coordinator's `is_authored_tool` discriminant + args source).
 */
export const TOOL_ARGS_KEY = "kx.tool.args";

/**
 * Serialize a flat tool-call arg map to the canonical-JSON string the three SDK
 * surfaces lower byte-identically: keys sorted ascending, compact separators, no
 * floats (SN-8 — the server schema is integer/bytes/bool/enum-typed).
 */
function canonicalArgsJson(args: Readonly<Record<string, string | number | boolean>>): string {
  const sorted: Record<string, string | number | boolean> = {};
  for (const k of Object.keys(args).sort()) {
    sorted[k] = args[k] as string | number | boolean;
  }
  return JSON.stringify(sorted);
}

/** Factories for the live `pure` / `model` / `tool` palette (TS has no operator overloading). */
export const task = {
  /** A `pure` step (optional string params). */
  pure(params: Readonly<Record<string, string>> = {}): Task {
    return new Task("pure", "", "", { ...params });
  },
  /**
   * A `model` step. Batch A: `modelId` is OPTIONAL — omit it (or pass `""`) and the
   * SERVER binds the served model (SN-8); set a client `defaultModel` to fill it, or
   * name a specific served model. `opts.reasoning` (`full`/`minimal`/`off`/`strip`)
   * sets the opt-in reasoning mode (absent ⇒ the model's own behavior + a byte-identical
   * MoteId). PR-9b (D161.1): pass `opts.tools` (an array of names → version `"1"`, or a
   * `{ name: version }` map) to make it a **deterministic-agentic step** — the model
   * runs a bounded reason→tool→observe loop over the granted tool SET (the same step
   * the string DSL authors as `handle@tool@tool`). `opts.maxTurns` / `maxToolCalls`
   * bound the loop (default 8 / 20 — decoupled, a turn may fire N tools; ignored with no tools).
   */
  model(
    modelId = "",
    prompt = "",
    params: Readonly<Record<string, string>> = {},
    opts: {
      tools?: readonly ToolRef[] | Readonly<Record<string, string>>;
      maxTurns?: number;
      maxToolCalls?: number;
      reasoning?: ReasoningMode;
      /** APP ONLY: catalog skill names bound to this step (see {@link Task.appSkills}). */
      skills?: readonly string[];
      /** APP ONLY: connection descriptors bound to this step. */
      connections?: readonly string[];
      /** APP ONLY: dataset names this step grounds on. */
      datasets?: readonly string[];
      /** APP ONLY: App handles this step calls (see {@link Task.appApps}). */
      apps?: readonly string[];
    } = {},
  ): Task {
    const stepParams: Record<string, string> = { ...params };
    if (opts.reasoning !== undefined) {
      stepParams[REASONING_KEY] = validateReasoning(opts.reasoning);
    }
    // V2b: `localTool(...)` defs ride off the contract (resolved at the run terminal);
    // string/record grants lower as before (golden-corpus byte-identical).
    const { strings, locals } = splitTools(opts.tools);
    return new Task(
      "model",
      modelId,
      prompt,
      stepParams,
      grantsToContract(strings),
      opts.maxTurns,
      opts.maxToolCalls,
      locals,
      [...(opts.skills ?? [])],
      [...(opts.connections ?? [])],
      [...(opts.datasets ?? [])],
      [...(opts.apps ?? [])],
    );
  },
  /**
   * A `tool` step (PR-6b-2): fire a single REGISTERED tool. `toolId` + `version`
   * name the tool the SERVER resolves (SN-8); `args` are the tool-call arguments,
   * lowered to one canonical-JSON object under {@link TOOL_ARGS_KEY}.
   */
  tool(
    toolId: string,
    version: string,
    args: Readonly<Record<string, string | number | boolean>> = {},
  ): Task {
    return new Task(
      "tool",
      "",
      "",
      { [TOOL_ARGS_KEY]: canonicalArgsJson(args) },
      {
        [toolId]: version,
      },
    );
  },
  /**
   * V2b: a standalone TOOL node firing a LOCAL `localTool(...)` function — the
   * contract is filled at the run terminal (the server-derived `<server>/<name>`).
   */
  localTool(
    def: LocalToolDef,
    args: Readonly<Record<string, string | number | boolean>> = {},
  ): Task {
    return new Task(
      "tool",
      "",
      "",
      { [TOOL_ARGS_KEY]: canonicalArgsJson(args) },
      {},
      undefined,
      undefined,
      [def],
    );
  },
};

/**
 * A composable sub-expression over the shared, ordered, deduped node set:
 * `{ entries, exits }` (the same node may appear in both — a single-node fragment).
 * A {@link Task} is a leaf fragment; {@link seq} / {@link par} / {@link group}
 * combine fragments. Internal to lowering; not constructed directly.
 */
interface Fragment {
  entries: Task[];
  exits: Task[];
}

/** A composed fragment, as produced by the combinator API (a `Task` is a leaf frag). */
export type Frag = Task | ChainFrag;

/** The opaque combinator-built fragment (wraps a closure that evaluates against a node accumulator). */
export class ChainFrag {
  /** @internal */
  constructor(readonly _eval: (acc: NodeAccumulator) => Fragment) {}
}

/** Order-preserving node registry — first-appearance index per distinct `Task`. */
class NodeAccumulator {
  readonly nodes: Task[] = [];
  private readonly seen = new Set<Task>();

  /** Register `t` on first appearance; the same object resolves to the same node. */
  register(t: Task): void {
    if (!this.seen.has(t)) {
      this.seen.add(t);
      this.nodes.push(t);
    }
  }

  index(t: Task): number {
    return this.nodes.indexOf(t);
  }
}

/** Evaluate a {@link Frag} (a leaf `Task` or a `ChainFrag`) against the accumulator. */
function evalFrag(frag: Frag, acc: NodeAccumulator): Fragment {
  if (frag instanceof Task) {
    acc.register(frag);
    return { entries: [frag], exits: [frag] };
  }
  return frag._eval(acc);
}

/** Order-preserving dedup of a task list (the parallel-merge `entries`/`exits` rule). */
function dedup(tasks: Task[]): Task[] {
  const out: Task[] = [];
  const seen = new Set<Task>();
  for (const t of tasks) {
    if (!seen.has(t)) {
      seen.add(t);
      out.push(t);
    }
  }
  return out;
}

/**
 * Sequential composition (left-folded). For adjacent fragments `A > B` add a DATA
 * edge from every `A.exits` to every `B.entries`; the fragment spans `A.entries`
 * → `B.exits`. Edges are accumulated on the {@link DslEdge} sink threaded via the
 * accumulator's owning lowering (see {@link lowerFrag}).
 */
export function seq(...frags: Frag[]): ChainFrag {
  if (frags.length === 0) {
    throw new ChainParseError("seq() needs at least one fragment");
  }
  return new ChainFrag((acc) => foldSeq(frags, acc));
}

/** Parallel merge (left-folded): no edges; entries/exits are the order-preserving union. */
export function par(...frags: Frag[]): ChainFrag {
  if (frags.length === 0) {
    throw new ChainParseError("par() needs at least one fragment");
  }
  return new ChainFrag((acc) => foldPar(frags, acc));
}

/** Grouping — identical to {@link par} (parallel merge); provided for readability. */
export function group(...frags: Frag[]): ChainFrag {
  return par(...frags);
}

// The edge sink is threaded through the accumulator: every fragment evaluation
// runs against the SAME accumulator, and `>` records edges into the shared list.
// We attach it to the accumulator so combinators stay pure closures.
interface EdgeRecordingAccumulator extends NodeAccumulator {
  edges?: Array<[Task, Task]>;
}

function foldSeq(frags: Frag[], acc: NodeAccumulator): Fragment {
  const sink = acc as EdgeRecordingAccumulator;
  if (sink.edges === undefined) {
    sink.edges = [];
  }
  const sub = frags.map((f) => evalFrag(f, acc));
  // Left-fold: for each adjacent pair, add the cartesian DATA edges.
  let cur = sub[0];
  if (cur === undefined) {
    throw new ChainParseError("seq() needs at least one fragment");
  }
  for (let i = 1; i < sub.length; i++) {
    const next = sub[i];
    if (next === undefined) {
      continue;
    }
    for (const parent of cur.exits) {
      for (const child of next.entries) {
        sink.edges.push([parent, child]);
      }
    }
    cur = { entries: cur.entries, exits: next.exits };
  }
  return cur;
}

function foldPar(frags: Frag[], acc: NodeAccumulator): Fragment {
  const sub = frags.map((f) => evalFrag(f, acc));
  const entries: Task[] = [];
  const exits: Task[] = [];
  for (const f of sub) {
    entries.push(...f.entries);
    exits.push(...f.exits);
  }
  return { entries: dedup(entries), exits: dedup(exits) };
}

// ---------------------------------------------------------------------------
// The string-DSL parser — a recursive descent matching the spec EBNF + precedence
// (tightest → loosest): `[ ]` > `>` (seq) > `&` (par) > `|` (par), all left-assoc.
// ---------------------------------------------------------------------------

type Token =
  | { t: "handle"; v: string; pos: number }
  | { t: ">"; pos: number }
  | { t: "&"; pos: number }
  | { t: "|"; pos: number }
  | { t: "["; pos: number }
  | { t: "]"; pos: number }
  | { t: "@"; pos: number };

const HANDLE_START = /[A-Za-z_]/;
const HANDLE_REST = /[A-Za-z0-9_-]/;

/** Tokenize a chain expression (whitespace insignificant). */
function tokenize(expr: string): Token[] {
  const toks: Token[] = [];
  let i = 0;
  while (i < expr.length) {
    const c = expr[i];
    if (c === undefined) {
      break;
    }
    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      i++;
      continue;
    }
    if (c === ">" || c === "&" || c === "|" || c === "[" || c === "]" || c === "@") {
      toks.push({ t: c, pos: i });
      i++;
      continue;
    }
    if (HANDLE_START.test(c)) {
      const start = i;
      i++;
      while (i < expr.length) {
        const r = expr[i];
        if (r === undefined || !HANDLE_REST.test(r)) {
          break;
        }
        i++;
      }
      toks.push({ t: "handle", v: expr.slice(start, i), pos: start });
      continue;
    }
    throw new ChainParseError(`unexpected character '${c}' at position ${i}`);
  }
  return toks;
}

/** A handle-leaf placeholder the parser emits before `tasks` resolution to a Task. */
class HandleRef {
  constructor(readonly handle: string) {}
}

/** A parse fragment over handle refs (resolved to `Task`s during lowering). */
interface ParseFrag {
  entries: HandleRef[];
  exits: HandleRef[];
}

/** Hand-rolled recursive-descent parser; emits combinator closures over handle refs. */
class Parser {
  private pos = 0;
  // The shared handle registry (first-appearance order, same HandleRef per handle).
  private readonly refs = new Map<string, HandleRef>();
  readonly order: HandleRef[] = [];
  // Accumulated `>` edges, by handle-ref pair (deduped + sorted at lowering).
  readonly edges: Array<[HandleRef, HandleRef]> = [];
  // PR-9b: per-handle `@`-tag grants (order-preserving, deduped), applied to the
  // resolved MODEL Task at lowering.
  readonly grants = new Map<string, string[]>();

  constructor(private readonly toks: Token[]) {}

  private peek(): Token | undefined {
    return this.toks[this.pos];
  }

  private next(): Token | undefined {
    return this.toks[this.pos++];
  }

  /** Resolve (or first-register) a handle to its shared {@link HandleRef}. */
  private ref(handle: string): HandleRef {
    let r = this.refs.get(handle);
    if (r === undefined) {
      r = new HandleRef(handle);
      this.refs.set(handle, r);
      this.order.push(r);
    }
    return r;
  }

  /** Parse the full chain; throws {@link ChainParseError} on trailing tokens. */
  parse(): ParseFrag {
    if (this.toks.length === 0) {
      throw new ChainParseError("empty chain expression");
    }
    const frag = this.orexpr();
    if (this.pos !== this.toks.length) {
      const tok = this.peek();
      throw new ChainParseError(`unexpected token at position ${tok?.pos ?? this.pos}`);
    }
    return frag;
  }

  // orexpr := andexpr ( "|" andexpr )*  — loosest, left-assoc.
  private orexpr(): ParseFrag {
    let left = this.andexpr();
    while (this.peek()?.t === "|") {
      this.next();
      left = this.mergePar(left, this.andexpr());
    }
    return left;
  }

  // andexpr := seqexpr ( "&" seqexpr )*  — tighter parallel, left-assoc.
  private andexpr(): ParseFrag {
    let left = this.seqexpr();
    while (this.peek()?.t === "&") {
      this.next();
      left = this.mergePar(left, this.seqexpr());
    }
    return left;
  }

  // seqexpr := atom ( ">" atom )*  — tightest binary (sequential), left-assoc.
  private seqexpr(): ParseFrag {
    let left = this.atom();
    while (this.peek()?.t === ">") {
      this.next();
      const right = this.atom();
      for (const parent of left.exits) {
        for (const child of right.entries) {
          this.edges.push([parent, child]);
        }
      }
      left = { entries: left.entries, exits: right.exits };
    }
    return left;
  }

  // atom := handle | "[" chain "]"  — `[ ]` is precedence-only (tightest).
  private atom(): ParseFrag {
    const tok = this.peek();
    if (tok === undefined) {
      throw new ChainParseError("unexpected end of expression (expected a handle or '[')");
    }
    if (tok.t === "handle") {
      this.next();
      const r = this.ref(tok.v);
      this.takeGrants(tok.v); // PR-9b: an optional `@tool@tool` suffix.
      return { entries: [r], exits: [r] };
    }
    if (tok.t === "[") {
      this.next();
      if (this.peek()?.t === "]") {
        throw new ChainParseError(`empty group '[]' at position ${tok.pos}`);
      }
      const inner = this.orexpr();
      const close = this.next();
      if (close?.t !== "]") {
        throw new ChainParseError(`unclosed group '[' opened at position ${tok.pos}`);
      }
      return inner; // brackets are precedence-only.
    }
    throw new ChainParseError(`expected a handle or '[' at position ${tok.pos}`);
  }

  // PR-9b: consume a `grants := ("@" handle)+` suffix on the just-parsed handle —
  // order-preserving deduped tool names recorded for lowering. A stray `@` with no
  // tool name is a parse error.
  private takeGrants(handle: string): void {
    const tags = this.grants.get(handle) ?? [];
    while (this.peek()?.t === "@") {
      this.next(); // consume the `@`
      const nxt = this.peek();
      if (nxt === undefined || nxt.t !== "handle") {
        throw new ChainParseError(
          `expected a tool name after '@' at position ${nxt?.pos ?? this.pos}`,
        );
      }
      this.next(); // consume the tool-name handle
      if (!tags.includes(nxt.v)) {
        tags.push(nxt.v);
      }
    }
    if (tags.length > 0) {
      this.grants.set(handle, tags);
    }
  }

  private mergePar(a: ParseFrag, b: ParseFrag): ParseFrag {
    return {
      entries: dedupRefs([...a.entries, ...b.entries]),
      exits: dedupRefs([...a.exits, ...b.exits]),
    };
  }
}

/** Order-preserving dedup of handle refs (the parallel-merge rule, on the parse side). */
function dedupRefs(refs: HandleRef[]): HandleRef[] {
  const out: HandleRef[] = [];
  const seen = new Set<HandleRef>();
  for (const r of refs) {
    if (!seen.has(r)) {
      seen.add(r);
      out.push(r);
    }
  }
  return out;
}

/** A canonical lowered step (the corpus snake_case shape; `params` values are strings). */
export interface LoweredStep {
  kind: "pure" | "model" | "tool";
  model_id: string;
  prompt: string;
  body_signature_id: null;
  tool_contract: Record<string, string>;
  params: Record<string, string>;
}

/** A canonical lowered edge (the corpus shape). */
export interface LoweredEdge {
  parent: number;
  child: number;
  edge: "data";
}

/** The canonical lowering — node-ordered steps + deduped, sorted edges (the corpus shape). */
export interface Lowered {
  steps: LoweredStep[];
  edges: LoweredEdge[];
  /**
   * PR-7b: the chain-level context-bundle handles, emitted verbatim (caller order
   * — the SERVER canonicalizes the sorted ref-set into each entry Mote at bind,
   * SN-8). Absent on the wire ⇒ `[]`; the corpus pins its byte-identity.
   */
  context_bundles: string[];
}

/** Sort + dedup edge index pairs ascending by (parent, child) — the canonical rule. */
function canonicalizeEdges(pairs: Array<[number, number]>): LoweredEdge[] {
  const seen = new Set<string>();
  const unique: Array<[number, number]> = [];
  for (const [p, c] of pairs) {
    const key = `${p}:${c}`;
    if (!seen.has(key)) {
      seen.add(key);
      unique.push([p, c]);
    }
  }
  unique.sort((x, y) => (x[0] !== y[0] ? x[0] - y[0] : x[1] - y[1]));
  return unique.map(([parent, child]) => ({ parent, child, edge: "data" }));
}

/** A {@link Task} → the canonical lowered step (display/test shape). */
function taskToLoweredStep(t: Task): LoweredStep {
  return {
    kind: t.kind,
    model_id: t.modelId,
    prompt: t.prompt,
    body_signature_id: null,
    tool_contract: { ...t.toolContract },
    params: effectiveParams(t),
  };
}

/**
 * PR-9b: derive a granted MODEL Task — merge `@`-tag tool names (version `"1"`)
 * into a copy of `base`'s tool_contract. A non-model base is a fail-closed
 * authoring error (the deterministic-agentic step requires a MODEL step).
 */
function withGrants(base: Task, handle: string, tags: string[]): Task {
  if (base.kind !== "model") {
    throw new ChainAgenticError(handle, base.kind);
  }
  const contract: Record<string, string> = { ...base.toolContract };
  for (const tag of tags) {
    if (!(tag in contract)) {
      contract[tag] = "1";
    }
  }
  return new Task(
    base.kind,
    base.modelId,
    base.prompt,
    { ...base.params },
    contract,
    base.maxTurns,
    base.maxToolCalls,
    base.localTools,
  );
}

/** Kahn topological check — reject a cycle / self-loop (`a > a`, `a>b | b>a`). */
function assertAcyclic(nodeCount: number, edges: LoweredEdge[]): void {
  const indegree = new Array<number>(nodeCount).fill(0);
  const adj: number[][] = Array.from({ length: nodeCount }, () => []);
  for (const e of edges) {
    // A self-loop is a cycle by definition.
    if (e.parent === e.child) {
      throw new ChainCycleError("chain expression has a self-loop");
    }
    const children = adj[e.parent];
    if (children !== undefined) {
      children.push(e.child);
    }
    indegree[e.child] = (indegree[e.child] ?? 0) + 1;
  }
  const queue: number[] = [];
  for (let i = 0; i < nodeCount; i++) {
    if (indegree[i] === 0) {
      queue.push(i);
    }
  }
  let visited = 0;
  while (queue.length > 0) {
    const n = queue.shift();
    if (n === undefined) {
      break;
    }
    visited++;
    for (const m of adj[n] ?? []) {
      const d = (indegree[m] ?? 0) - 1;
      indegree[m] = d;
      if (d === 0) {
        queue.push(m);
      }
    }
  }
  if (visited !== nodeCount) {
    throw new ChainCycleError();
  }
}

/** Lower a parsed handle-DSL against a `tasks` map → node-ordered `Task`s + edge pairs. */
function lowerParse(
  parser: Parser,
  tasks: Readonly<Record<string, Task>>,
): { nodes: Task[]; edgePairs: Array<[number, number]> } {
  // Resolve every parsed handle to its Task (first-appearance order preserved).
  const refToIndex = new Map<HandleRef, number>();
  const nodes: Task[] = [];
  for (const ref of parser.order) {
    const base = tasks[ref.handle];
    if (base === undefined) {
      throw new ChainUnknownHandleError(ref.handle);
    }
    // PR-9b: apply any `@`-tag grants for this handle to a derived MODEL Task.
    const tags = parser.grants.get(ref.handle);
    const t = tags !== undefined && tags.length > 0 ? withGrants(base, ref.handle, tags) : base;
    refToIndex.set(ref, nodes.length);
    nodes.push(t);
  }
  const edgePairs: Array<[number, number]> = parser.edges.map(([p, c]) => {
    const pi = refToIndex.get(p);
    const ci = refToIndex.get(c);
    if (pi === undefined || ci === undefined) {
      // Unreachable: every edge ref was registered in `order`.
      throw new ChainParseError("internal: edge references an unregistered handle");
    }
    return [pi, ci];
  });
  return { nodes, edgePairs };
}

/** Options for {@link chain}. */
export interface ChainOptions {
  /** The handle → {@link Task} resolution map (defined-but-unused tasks are ignored). */
  tasks: Readonly<Record<string, Task>>;
  /** The chain seed (default `0`). */
  seed?: number;
  /**
   * PR-7b: context-bundle handles to attach to the run (chain-level grounding the
   * server injects into every entry Mote; verbatim order). Also settable fluently
   * via {@link Chain.context}.
   */
  context?: readonly string[];
}

/**
 * A lowered chain — node-ordered steps + canonical edges, ready to {@link Chain.build}
 * into a `SubmitWorkflowRequest` (via the {@link BlueprintBuilder}) or {@link Chain.lower}
 * to the canonical corpus shape (display/parity tests). Immutable after construction.
 */
export class Chain {
  /** @internal — build via {@link chain} or {@link chainOf}. */
  constructor(
    private readonly nodes: readonly Task[],
    private readonly edgePairs: readonly LoweredEdge[],
    readonly seed: number,
    /** PR-7b: chain-level context-bundle handles (verbatim caller order). */
    readonly contextBundles: readonly string[] = [],
  ) {}

  /**
   * Attach context-bundle `handles` to this chain (PR-7b), returning a NEW
   * {@link Chain} (immutable — this one is unchanged). Repeated calls APPEND in
   * order; the SERVER resolves each handle to its content-refs and folds the
   * sorted set into every entry Mote's identity-bearing config, so a different
   * attached context ⇒ a different run (exactly-once-per-input+context). Context
   * is request-level — it attaches to the ENTRY Motes regardless of position;
   * there is no `context` step.
   */
  context(...handles: string[]): Chain {
    return new Chain(this.nodes, this.edgePairs, this.seed, [...this.contextBundles, ...handles]);
  }

  /**
   * The canonical lowering (the corpus snake_case shape; `params` values are strings).
   *
   * @example
   * ```ts
   * const low = chain("a > b", { tasks: { a: task.model("", "hi"), b: task.pure() } }).lower();
   * low.steps.map((s) => s.kind); // ["model", "pure"]
   * low.edges;                    // [{ parent: 0, child: 1, edge: "data" }]
   * ```
   */
  lower(): Lowered {
    return {
      steps: this.nodes.map((t) => taskToLoweredStep(t)),
      edges: this.edgePairs.map((e) => ({ ...e })),
      context_bundles: [...this.contextBundles],
    };
  }

  /** The distinct {@link LocalToolDef}s referenced across this chain's steps (V2b) —
   * what the run terminal registers + resolves. */
  collectLocalTools(): LocalToolDef[] {
    const seen = new Set<LocalToolDef>();
    for (const t of this.nodes) {
      for (const lt of t.localTools) {
        seen.add(lt);
      }
    }
    return [...seen];
  }

  /**
   * The `SubmitWorkflowRequest` init shape, assembled via the EXISTING
   * {@link BlueprintBuilder} (kinds → enum, strings → UTF-8, mode → FROZEN).
   * The DSL only feeds the builder `StepInput`/`EdgeInput` lists — it never
   * reassembles the wire request itself.
   *
   * V2b: `resolved` maps each local-tool def → its server-derived `<server>/<name>`;
   * those names are folded into the owning step's `tool_contract` at build time
   * (absent ⇒ a chain with no local tools, byte-identical to before).
   */
  build(
    resolved?: ReadonlyMap<LocalToolDef, string>,
  ): MessageInitShape<typeof SubmitWorkflowRequestSchema> {
    const builder = new BlueprintBuilder(this.seed);
    for (let i = 0; i < this.nodes.length; i++) {
      const t = this.nodes[i];
      if (t === undefined) continue;
      refuseAppBindingsOnWorkflow(i, t);
      const step = t.toStepInput();
      if (resolved !== undefined && t.localTools.length > 0) {
        const contract: Record<string, string> = { ...step.toolContract };
        for (const lt of t.localTools) {
          const name = resolved.get(lt);
          if (name !== undefined && !(name in contract)) {
            contract[name] = lt.version;
          }
        }
        step.toolContract = contract;
      }
      builder.addStep(step);
    }
    for (const e of this.edgePairs) {
      const edge: EdgeInput = { parent: e.parent, child: e.child, edge: "data" };
      builder.addEdge(edge);
    }
    builder.contextBundles(this.contextBundles);
    return builder.build();
  }

  // --- Batch B (D161.2): portable blueprint export / import -----------------

  /**
   * Export this chain as a PORTABLE blueprint object — the same shape
   * `kx blueprint run --file` and {@link Chain.fromBlueprint} consume. Round-trips:
   * feeding it back to {@link Chain.fromBlueprint} (or the CLI) re-compiles to the
   * IDENTICAL `SubmitWorkflowRequest` as {@link Chain.build}. `params` are in their
   * FOLDED form (a tool step's args under `kx.tool.args`; an agentic MODEL step's
   * budget under `max_turns`/`max_tool_calls`) — import is fold-idempotent. `model_id`
   * stays as authored (empty ⇒ the server binds the served model, SN-8) so the
   * artifact is portable across serves. Each `kind` is explicit (self-describing).
   */
  toBlueprint(): DagSpecJson {
    const low = this.lower();
    // `lower()` returns wire StepInputs, which do not carry the App bindings — read those
    // off the source Tasks by position (same order). The App path preserves them in the
    // blueprint JSON; the workflow path never reaches here with one set.
    const tasks = this.nodes;
    const steps: DagSpecStep[] = low.steps.map((s, i) => {
      const step: DagSpecStep = { kind: s.kind };
      if (s.model_id) step.model_id = s.model_id;
      if (s.prompt) step.prompt = s.prompt;
      if (Object.keys(s.tool_contract).length > 0) step.tool_contract = s.tool_contract;
      if (Object.keys(s.params).length > 0) step.params = s.params;
      const t = tasks[i];
      // Emitted only when non-empty, so a chain that binds nothing produces byte-identical
      // blueprint JSON to one authored before these existed.
      if (t && t.appSkills.length > 0) step.skills = [...t.appSkills];
      if (t && t.appConnections.length > 0) step.connections = [...t.appConnections];
      if (t && t.appDatasets.length > 0) step.datasets = [...t.appDatasets];
      if (t && t.appApps.length > 0) step.apps = [...t.appApps];
      return step;
    });
    const bp: DagSpecJson = { seed: this.seed, execution_mode: "frozen", steps };
    if (low.edges.length > 0) {
      bp.edges = low.edges.map((e) => ({ parent: e.parent, child: e.child, edge: "data" }));
    }
    if (this.contextBundles.length > 0) bp.context_bundles = [...this.contextBundles];
    return bp;
  }

  /**
   * Write {@link Chain.toBlueprint} as pretty JSON to `path` — the portable artifact
   * (save / version / share; re-run with `kx blueprint run --file` or
   * {@link Chain.fromBlueprint}). NODE-ONLY: the dynamic `node:fs/promises` import
   * keeps it out of the `web`/`chains` static bundle graph (a browser cannot write a
   * file — author there, export from Node).
   */
  async export(path: string): Promise<void> {
    const fs = await import("node:fs/promises");
    await fs.writeFile(path, `${JSON.stringify(this.toBlueprint(), null, 2)}\n`);
  }

  /**
   * Compile a portable blueprint object (from {@link Chain.toBlueprint}, the CLI
   * `--emit-blueprint`, or a hand-authored DAG) into a `SubmitWorkflowRequest` init
   * shape ready for `client.submitWorkflow`. Accepts BOTH artifact forms: the SDK
   * FOLDED form (args/budget already in `params`) and the CLI ARGS-SEPARATED form (a
   * tool step's `args` map + an agentic step's `max_turns`/`max_tool_calls` fields) —
   * both fold to the same request. The chain TOPOLOGY is not recoverable from a DAG
   * (only the request is), so this returns the request, not a {@link Chain}.
   */
  static fromBlueprint(spec: DagSpecJson): MessageInitShape<typeof SubmitWorkflowRequestSchema> {
    const builder = new BlueprintBuilder(spec.seed ?? 0);
    (spec.steps ?? []).forEach((d, i) => {
      refuseSpecAppBindings(i, d);
      builder.addStep(stepFromSpec(d));
    });
    for (const e of spec.edges ?? []) {
      builder.addEdge({
        parent: e.parent,
        child: e.child,
        edge: e.edge === "control" ? "control" : "data",
        nonCascade: e.non_cascade ?? false,
      });
    }
    builder.contextBundles(spec.context_bundles ?? []);
    if (spec.execution_mode === "dynamic") builder.mode("dynamic");
    return builder.build();
  }

  /** Read a portable blueprint JSON file (Node) and compile it (see
   * {@link Chain.fromBlueprint}). */
  static async fromBlueprintFile(
    path: string,
  ): Promise<MessageInitShape<typeof SubmitWorkflowRequestSchema>> {
    const fs = await import("node:fs/promises");
    const raw = await fs.readFile(path, "utf-8");
    return Chain.fromBlueprint(JSON.parse(raw) as DagSpecJson);
  }
}

/** One step in a portable blueprint (the `kx blueprint run --file` JSON shape). `kind`
 * is optional on IMPORT (inferred from fields, like the CLI); {@link Chain.toBlueprint}
 * always sets it. `args` / `max_turns` / `max_tool_calls` are the CLI ARGS-SEPARATED
 * form; the SDK export folds them into `params` instead (both import-equivalent). */
export interface DagSpecStep {
  kind?: StepKind;
  model_id?: string;
  prompt?: string;
  body_signature_id?: string | null;
  tool_contract?: Record<string, string>;
  params?: Record<string, string>;
  args?: Record<string, string | number | boolean>;
  max_turns?: number;
  max_tool_calls?: number;
  /**
   * APP ONLY — the catalog SKILL names bound to this step, naming entries in the App
   * envelope's `references.skills[].name`.
   *
   * This and the two below are BINDINGS, not declarations: `references` still holds the
   * skill's instructions ref, the credential name and the corpus; the step says only which
   * of them it uses. `RunApp` resolves them, and a capability NO step names binds where it
   * always did — the entry step — so a blueprint that binds nothing behaves exactly as
   * before and compiles to the same identity.
   *
   * A plain `SubmitWorkflow` has no `references` to name into, so lowering one of these as a
   * workflow is refused rather than silently dropped.
   */
  skills?: string[];
  /** APP ONLY — connection DESCRIPTORS bound to this step (`references.connections[]`). */
  connections?: string[];
  /** APP ONLY — DATASET names this step grounds on (`references.datasets[]`). */
  datasets?: string[];
  /** APP ONLY — App HANDLES this step calls (`references.apps[].handle`). Each is lowered
   * into the run as its own sub-graph, feeding this step. */
  apps?: string[];
}

/** A portable blueprint (the `kx blueprint run --file` JSON shape). */
export interface DagSpecJson {
  seed: number;
  execution_mode?: string;
  steps: DagSpecStep[];
  edges?: Array<{ parent: number; child: number; edge?: string; non_cascade?: boolean }>;
  context_bundles?: string[];
}

/** Infer a step's kind from field presence when `kind` is omitted — mirrors the CLI
 * `StepSpec::resolve_kind` (model fields win; then a tool contract; else pure). */
function inferKind(d: DagSpecStep): StepKind {
  if (d.model_id || d.prompt) return "model";
  if (d.tool_contract && Object.keys(d.tool_contract).length > 0) return "tool";
  return "pure";
}

/** One blueprint step object → a {@link StepInput}, folding the CLI args-separated form
 * (a `args` map ⇒ `kx.tool.args`; `max_turns`/`max_tool_calls` ⇒ the budget keys) so
 * both artifact forms import to the same request. */
/** Refuse an App-envelope capability binding on the WORKFLOW lowering path — a
 *  `SubmitWorkflow` has no `references` for a `skills`/`connections`/`datasets` NAME to
 *  point at, so the runtime could only drop it. Fails at authoring with a message that says
 *  where the field IS honoured — mirroring the Rust `kx_blueprint::to_request` refusal, so
 *  all three surfaces agree. */
function refuseAppBindingsOnWorkflow(index: number, t: Task): void {
  if (!t.hasAppBindings()) return;
  const named = [
    t.appSkills.length > 0 ? "skills" : null,
    t.appConnections.length > 0 ? "connections" : null,
    t.appDatasets.length > 0 ? "datasets" : null,
    t.appApps.length > 0 ? "apps" : null,
  ].filter((x): x is string => x !== null);
  throw new ChainParseError(
    `step ${index} declares ${named.join(" + ")} — a per-step capability list is an App-envelope binding that names an entry in the App's references, and RunApp is what resolves it. A workflow has no references to name into: author this as an App (app(...)), or grant the step a tool directly with { tools: [...] }.`,
  );
}

/** As {@link refuseAppBindingsOnWorkflow}, over a parsed blueprint step. */
function refuseSpecAppBindings(index: number, d: DagSpecStep): void {
  const named = [
    d.skills && d.skills.length > 0 ? "skills" : null,
    d.connections && d.connections.length > 0 ? "connections" : null,
    d.datasets && d.datasets.length > 0 ? "datasets" : null,
    d.apps && d.apps.length > 0 ? "apps" : null,
  ].filter((x): x is string => x !== null);
  if (named.length === 0) return;
  throw new ChainParseError(
    `step ${index} declares ${named.join(" + ")} — a per-step capability list is an App-envelope binding resolved by RunApp; a workflow blueprint has no references to name into. Author it as an App, or grant the step a tool directly via tool_contract.`,
  );
}

function stepFromSpec(d: DagSpecStep): StepInput {
  const kind = d.kind ?? inferKind(d);
  const params: Record<string, Uint8Array | string> = { ...(d.params ?? {}) };
  if (d.args && Object.keys(d.args).length > 0) {
    params[TOOL_ARGS_KEY] = canonicalArgsJson(d.args);
  }
  if (kind === "model" && d.tool_contract && Object.keys(d.tool_contract).length > 0) {
    if (d.max_turns !== undefined) params[REACT_MAX_TURNS_KEY] = String(d.max_turns);
    if (d.max_tool_calls !== undefined) params[REACT_MAX_TOOL_CALLS_KEY] = String(d.max_tool_calls);
  }
  return {
    kind,
    modelId: d.model_id ?? "",
    prompt: d.prompt ?? "",
    bodySignatureId: d.body_signature_id ?? undefined,
    toolContract: d.tool_contract ?? {},
    params,
  };
}

/** Assemble a {@link Chain} from resolved node-ordered tasks + canonical edges + seed. */
function chainOf(
  nodes: Task[],
  edgePairs: Array<[number, number]>,
  seed: number,
  contextBundles: readonly string[] = [],
): Chain {
  const edges = canonicalizeEdges(edgePairs);
  assertAcyclic(nodes.length, edges);
  return new Chain(nodes, edges, seed, contextBundles);
}

/**
 * Parse a chain expression (the exact spec grammar) + lower it against `opts.tasks`
 * to a {@link Chain}. Throws {@link ChainParseError} (empty / malformed / empty
 * group), {@link ChainUnknownHandleError} (a parsed handle absent from `tasks`),
 * or {@link ChainCycleError} (a cycle / self-loop via handle reuse).
 *
 * ```ts
 * const c = chain("a > [b & c]", { tasks: { a: task.pure(), b: task.pure(), c: task.pure() } });
 * await kx.runChain(c, { wait: true });
 * ```
 */
export function chain(expr: string, opts: ChainOptions): Chain {
  const parser = new Parser(tokenize(expr));
  parser.parse();
  const { nodes, edgePairs } = lowerParse(parser, opts.tasks);
  return chainOf(nodes, edgePairs, opts.seed ?? 0, opts.context ?? []);
}

/**
 * Lower a combinator-built fragment ({@link seq} / {@link par} / {@link group} /
 * a leaf {@link Task}) to a {@link Chain} — the type-safe alternative to the string
 * DSL (TS has no operator overloading). Parity with {@link chain} is pinned by the
 * combinator tests. Reusing the same `Task` object reuses the same node.
 *
 * ```ts
 * const a = task.pure(), b = task.pure(), c = task.pure();
 * chainFrom(seq(a, par(b, c)));          // == chain("a > [b & c]", ...)
 * ```
 */
export function chainFrom(
  frag: Frag,
  opts: { seed?: number; context?: readonly string[] } = {},
): Chain {
  const acc = new NodeAccumulator() as EdgeRecordingAccumulator;
  evalFrag(frag, acc);
  const edgePairs: Array<[number, number]> = (acc.edges ?? []).map(([p, c]) => [
    acc.index(p),
    acc.index(c),
  ]);
  return chainOf([...acc.nodes], edgePairs, opts.seed ?? 0, opts.context ?? []);
}
