/**
 * The visual builder's pure graph model + validation + request mapping (no React,
 * no reactflow) — the single source of the authored DAG's structure, kept isolated
 * so every topology is exhaustively unit-testable. Mirrors the Rust core's
 * pure/total/testable discipline (and the live-DAG `dag-graph.ts`).
 *
 * SN-8: the builder NEVER computes a MoteId or a warrant. `toRequest` assembles
 * ONLY the topology + params the SERVER compiles + admits (via the SDK
 * `BlueprintBuilder`). A tampered client DAG changes only what is PROPOSED, never
 * the identity it is assigned. The palette is PURE / MODEL — no REACT/tool-grant
 * authoring (that would reopen the SubmitWorkflow admission boundary, §2.171).
 */

import type { StepInput } from "@kortecx/sdk/web";
import { BlueprintBuilder, task } from "@kortecx/sdk/web";

/** The authored step palette (EXEC is reserved server-side; the UI offers
 *  PURE / MODEL / TOOL). TOOL (PR-6b-2) fires a single REGISTERED tool: the SERVER
 *  resolves it in the live registry + builds the per-step warrant (client tool_grants
 *  stay refused — SN-8/§2.171), so adding it does NOT reopen the admission boundary. */
export type BuilderStepKind = "pure" | "model" | "tool";

/** One authored builder step. `params` is the JSON-OBJECT TEXT the user edits in
 *  Monaco (parsed at submit); `prompt`/`modelId` apply to MODEL steps; `toolId`/
 *  `toolVersion` + the params-as-args apply to TOOL steps. */
export interface BuilderStep {
  /** Client-local node id (NOT a MoteId — the server derives identity). */
  readonly id: string;
  readonly kind: BuilderStepKind;
  /** Human label (display only). */
  readonly label: string;
  /** MODEL: the served model id (must equal a served model, validated server-side). */
  readonly modelId: string;
  /** MODEL: the prompt (Monaco). */
  readonly prompt: string;
  /** Free-param JSON-object text (parsed to bytes server-side). For a TOOL step this
   *  JSON object IS the tool-call arguments (lowered to the canonical TOOL_ARGS_KEY blob). */
  readonly paramsText: string;
  /** Optional reasoning-mode (PR-4 Phase F) — opt-in MODEL knob; "" ⇒ default. */
  readonly reasoning: "" | "full" | "minimal" | "off";
  /** TOOL: the registered tool id (from `DiscoverTools`); the SERVER resolves it. */
  readonly toolId: string;
  /** TOOL: the registered tool version (defaults to "1" when blank). */
  readonly toolVersion: string;
  /** MODEL (PR-9b-2b): the author-declared tool-grant SET `{tool_id: version}` that
   *  makes this a DETERMINISTIC-AGENTIC step — a bounded reason→tool→observe loop over
   *  the FIXED set (the set is part of the step's identity). Empty ⇒ a plain model step
   *  (byte-identical to before). The SERVER builds the union warrant + drives the loop
   *  (SN-8: client tool_grants stay refused). */
  readonly toolContract: Readonly<Record<string, string>>;
  /** MODEL agentic: the per-step model-turn budget (default 8; `0 < maxTurns ≤ 8`). */
  readonly maxTurns?: number;
  /** MODEL agentic: the per-step total tool-call budget (default 20, ceiling 20 —
   *  decoupled from maxTurns at T-MULTI-ELEMENT-TOOLCALLS; a turn can fire N tools). */
  readonly maxToolCalls?: number;
}

/** One authored edge (by client-local node id). `instruction` (D141.5) is the
 *  inter-step instruction-file text; in Tier-1 it folds into the downstream MODEL
 *  step's prompt at submit (its content-bundle backing arrives with PR-7). */
export interface BuilderEdge {
  readonly id: string;
  readonly source: string;
  readonly target: string;
  readonly edge: "data" | "control";
  readonly instruction: string;
}

export interface BuilderGraph {
  readonly steps: readonly BuilderStep[];
  readonly edges: readonly BuilderEdge[];
}

/** A stable hash of the builder *topology* (sorted node ids + sorted edges) —
 *  the no-thrash layout key (excludes labels/prompts/params so editing a node's
 *  config never relayouts the canvas). */
export function builderTopologyHash(graph: BuilderGraph): string {
  const ids = graph.steps.map((s) => s.id).sort();
  const edges = graph.edges.map((e) => `${e.source}>${e.target}:${e.edge}`).sort();
  return `${ids.join(",")}|${edges.join(",")}`;
}

export interface AcyclicResult {
  readonly ok: boolean;
  /** The node ids left in a cycle (non-empty iff `ok` is false). */
  readonly cycle: readonly string[];
}

/**
 * Client-side acyclicity precheck (Kahn) — a UX guard mirroring the server's
 * `kx_workflow::compile` `topo_order`; the SERVER stays the authority (a cyclic
 * DAG is refused at admission regardless). Also flags an edge to a missing node.
 */
export function validateAcyclic(graph: BuilderGraph): AcyclicResult {
  const ids = new Set(graph.steps.map((s) => s.id));
  const indegree = new Map<string, number>();
  const children = new Map<string, string[]>();
  for (const s of graph.steps) {
    indegree.set(s.id, 0);
    children.set(s.id, []);
  }
  for (const e of graph.edges) {
    if (!ids.has(e.source) || !ids.has(e.target)) {
      continue; // dangling edge — ignored by layout/submit; not a cycle
    }
    indegree.set(e.target, (indegree.get(e.target) ?? 0) + 1);
    children.get(e.source)?.push(e.target);
  }
  // Deterministic min-id frontier (matches the server's stable order).
  const frontier = [...ids].filter((id) => (indegree.get(id) ?? 0) === 0).sort();
  let processed = 0;
  while (frontier.length > 0) {
    const id = frontier.shift() as string;
    processed += 1;
    for (const c of (children.get(id) ?? []).sort()) {
      const d = (indegree.get(c) ?? 0) - 1;
      indegree.set(c, d);
      if (d === 0) {
        frontier.push(c);
        frontier.sort();
      }
    }
  }
  if (processed === ids.size) {
    return { ok: true, cycle: [] };
  }
  // Whatever still has indegree > 0 participates in (or descends from) a cycle.
  const cycle = [...ids].filter((id) => (indegree.get(id) ?? 0) > 0).sort();
  return { ok: false, cycle };
}

/** A human reason a graph cannot be submitted, or `null` when it is valid. */
export function validationError(graph: BuilderGraph): string | null {
  if (graph.steps.length === 0) {
    return "Add at least one step.";
  }
  for (const s of graph.steps) {
    if (s.kind === "model" && s.modelId.trim() === "") {
      return `Agent step "${s.label}" needs a model.`;
    }
    if (s.kind === "tool" && s.toolId.trim() === "") {
      return `Tool step "${s.label}" needs a registered tool.`;
    }
    // PR-9b-2b: a deterministic-agentic step's budget must satisfy the loop invariant
    // (mirrors the server's `react_seed_params` + the SDK lowering gate). Defaults
    // (8 / 6) are valid; an explicit out-of-range pair is refused at authoring.
    if (s.kind === "model" && Object.keys(s.toolContract).length > 0) {
      const turns = s.maxTurns ?? 8;
      const calls = s.maxToolCalls ?? 6;
      if (calls < 1 || calls >= turns || turns > 8) {
        return `Agent step "${s.label}" tool budget must satisfy 0 < tool-calls < turns ≤ 8.`;
      }
    }
    if (s.paramsText.trim() !== "" && !isJsonObject(s.paramsText)) {
      return `Step "${s.label}" ${s.kind === "tool" ? "args" : "params"} must be a JSON object.`;
    }
  }
  const acyclic = validateAcyclic(graph);
  if (!acyclic.ok) {
    return `The graph has a cycle (${acyclic.cycle.length} step(s)); a workflow DAG must be acyclic.`;
  }
  return null;
}

/** `true` iff `text` parses to a non-array JSON object (or is blank). */
export function isJsonObject(text: string): boolean {
  if (text.trim() === "") {
    return true;
  }
  try {
    const v: unknown = JSON.parse(text);
    return v !== null && typeof v === "object" && !Array.isArray(v);
  } catch {
    return false;
  }
}

/**
 * Lower a builder graph to a `SubmitWorkflow` request (via the SDK
 * `BlueprintBuilder` — topology + params ONLY). Steps map by `addStep` index;
 * edges reference those indices. An edge instruction (D141.5) is PREPENDED to the
 * downstream MODEL step's prompt (the Tier-1 backing) so the instruction genuinely
 * reaches the agent. Throws `validationError` if the graph is not submittable.
 */
export function toRequest(graph: BuilderGraph, seed = 0) {
  const err = validationError(graph);
  if (err) {
    throw new Error(err);
  }
  const builder = new BlueprintBuilder(seed);
  const index = new Map<string, number>();
  // Per-step instruction prefix gathered from inbound edges (deterministic order).
  const inbound = new Map<string, string[]>();
  for (const e of [...graph.edges].sort((a, b) => a.id.localeCompare(b.id))) {
    if (e.instruction.trim() !== "") {
      const list = inbound.get(e.target) ?? [];
      list.push(e.instruction.trim());
      inbound.set(e.target, list);
    }
  }
  for (const s of graph.steps) {
    let step: StepInput;
    if (s.kind === "tool") {
      // Reuse the SDK `task.tool` factory so the canonical TOOL_ARGS_KEY lowering
      // is byte-identical to the Py/TS/CLI surfaces (the golden-corpus contract).
      step = task
        .tool(s.toolId, s.toolVersion.trim() === "" ? "1" : s.toolVersion, toolArgs(s))
        .toStepInput();
    } else if (s.kind === "model" && Object.keys(s.toolContract).length > 0) {
      // PR-9b-2b: a DETERMINISTIC-AGENTIC step — reuse the SDK `task.model(tools=,
      // maxTurns=, maxToolCalls=)` factory so the tool_contract + budget→params
      // lowering is BYTE-IDENTICAL to the chains DSL / CLI (the golden corpus). The
      // server builds the union warrant + parks/drives the bounded loop.
      let prompt = s.prompt;
      const instr = inbound.get(s.id);
      if (instr && instr.length > 0) {
        prompt = `${instr.join("\n\n")}\n\n${s.prompt}`.trim();
      }
      step = task
        .model(s.modelId, prompt, paramsRecord(s), {
          tools: s.toolContract,
          maxTurns: s.maxTurns,
          maxToolCalls: s.maxToolCalls,
        })
        .toStepInput();
    } else {
      const params = paramsRecord(s);
      let prompt = s.prompt;
      if (s.kind === "model") {
        const instr = inbound.get(s.id);
        if (instr && instr.length > 0) {
          prompt = `${instr.join("\n\n")}\n\n${s.prompt}`.trim();
        }
      }
      step = {
        kind: s.kind,
        modelId: s.kind === "model" ? s.modelId : undefined,
        prompt: s.kind === "model" ? prompt : undefined,
        params,
      };
    }
    index.set(s.id, builder.addStep(step));
  }
  for (const e of graph.edges) {
    const parent = index.get(e.source);
    const child = index.get(e.target);
    if (parent === undefined || child === undefined) {
      continue; // dangling — skip (validation already passed)
    }
    builder.addEdge({ parent, child, edge: e.edge });
  }
  return builder.build();
}

/** Parse a step's params text (+ the opt-in reasoning knob, PR-4 Phase F) into the
 *  string-valued params record the SDK encodes to bytes. */
function paramsRecord(s: BuilderStep): Record<string, string> {
  const out: Record<string, string> = {};
  if (s.paramsText.trim() !== "") {
    const parsed = JSON.parse(s.paramsText) as Record<string, unknown>;
    for (const [k, v] of Object.entries(parsed)) {
      out[k] = typeof v === "string" ? v : JSON.stringify(v);
    }
  }
  // The reasoning-mode is an opt-in declared free-param (default-unset ⇒ omitted,
  // so the step's identity is byte-identical to a no-reasoning step — SN-8/digest).
  if (s.kind === "model" && s.reasoning !== "") {
    out.reasoning = s.reasoning;
  }
  return out;
}

/** Parse a TOOL step's params text as its tool-call argument map (string/number/
 *  boolean values — no floats; the server schema is integer/bytes/bool/enum-typed).
 *  Blank ⇒ the empty `{}` call. Validation already proved it is a JSON object. */
function toolArgs(s: BuilderStep): Record<string, string | number | boolean> {
  if (s.paramsText.trim() === "") {
    return {};
  }
  const parsed = JSON.parse(s.paramsText) as Record<string, unknown>;
  const out: Record<string, string | number | boolean> = {};
  for (const [k, v] of Object.entries(parsed)) {
    out[k] =
      typeof v === "string" || typeof v === "number" || typeof v === "boolean"
        ? v
        : JSON.stringify(v);
  }
  return out;
}

/** A label for a fresh step of `kind`. */
function defaultLabel(kind: BuilderStepKind): string {
  if (kind === "model") {
    return "Agent";
  }
  return kind === "tool" ? "Tool" : "Step";
}

/** A fresh empty step of `kind` with a unique client-local id. */
export function newStep(kind: BuilderStepKind, id: string): BuilderStep {
  return {
    id,
    kind,
    label: defaultLabel(kind),
    modelId: "",
    prompt: "",
    paramsText: "",
    reasoning: "",
    toolId: "",
    toolVersion: "",
    toolContract: {},
  };
}
