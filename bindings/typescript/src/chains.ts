/**
 * The Chains DSL — compose a vetted palette of task handles (`pure` / `model`
 * today) into a Tier-1 DAG via a string expression OR a combinator API, then
 * lower to the EXISTING {@link BlueprintBuilder} for `SubmitWorkflow`. Kept in
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
import { BlueprintBuilder, type EdgeInput, type StepInput } from "./blueprints.js";
import type { SubmitWorkflowRequestSchema } from "./gen/kortecx/v1/gateway_pb.js";

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
    /** TOOL only (PR-6b-2): the single `{ tool_id: tool_version }` the step fires. */
    readonly toolContract: Readonly<Record<string, string>> = {},
  ) {}

  /** The {@link StepInput} this task lowers to (verbatim, the builder encodes params). */
  toStepInput(): StepInput {
    return {
      kind: this.kind,
      modelId: this.modelId,
      prompt: this.prompt,
      toolContract: { ...this.toolContract },
      params: { ...this.params },
    };
  }
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
  /** A `model` step: the model id + prompt (+ optional string params). */
  model(modelId: string, prompt: string, params: Readonly<Record<string, string>> = {}): Task {
    return new Task("model", modelId, prompt, { ...params });
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
  | { t: "]"; pos: number };

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
    if (c === ">" || c === "&" || c === "|" || c === "[" || c === "]") {
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
    params: { ...t.params },
  };
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
    const t = tasks[ref.handle];
    if (t === undefined) {
      throw new ChainUnknownHandleError(ref.handle);
    }
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
  ) {}

  /** The canonical lowering (the corpus snake_case shape; `params` values are strings). */
  lower(): Lowered {
    return {
      steps: this.nodes.map((t) => taskToLoweredStep(t)),
      edges: this.edgePairs.map((e) => ({ ...e })),
    };
  }

  /**
   * The `SubmitWorkflowRequest` init shape, assembled via the EXISTING
   * {@link BlueprintBuilder} (kinds → enum, strings → UTF-8, mode → FROZEN).
   * The DSL only feeds the builder `StepInput`/`EdgeInput` lists — it never
   * reassembles the wire request itself.
   */
  build(): MessageInitShape<typeof SubmitWorkflowRequestSchema> {
    const builder = new BlueprintBuilder(this.seed);
    for (const t of this.nodes) {
      builder.addStep(t.toStepInput());
    }
    for (const e of this.edgePairs) {
      const edge: EdgeInput = { parent: e.parent, child: e.child, edge: "data" };
      builder.addEdge(edge);
    }
    return builder.build();
  }
}

/** Assemble a {@link Chain} from resolved node-ordered tasks + canonical edges + seed. */
function chainOf(nodes: Task[], edgePairs: Array<[number, number]>, seed: number): Chain {
  const edges = canonicalizeEdges(edgePairs);
  assertAcyclic(nodes.length, edges);
  return new Chain(nodes, edges, seed);
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
  return chainOf(nodes, edgePairs, opts.seed ?? 0);
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
export function chainFrom(frag: Frag, opts: { seed?: number } = {}): Chain {
  const acc = new NodeAccumulator() as EdgeRecordingAccumulator;
  evalFrag(frag, acc);
  const edgePairs: Array<[number, number]> = (acc.edges ?? []).map(([p, c]) => [
    acc.index(p),
    acc.index(c),
  ]);
  return chainOf([...acc.nodes], edgePairs, opts.seed ?? 0);
}
