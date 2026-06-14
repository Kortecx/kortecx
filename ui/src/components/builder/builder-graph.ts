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
import { BlueprintBuilder } from "@kortecx/sdk/web";

/** The authored step palette (EXEC is reserved server-side; the UI offers PURE/MODEL). */
export type BuilderStepKind = "pure" | "model";

/** One authored builder step. `params` is the JSON-OBJECT TEXT the user edits in
 *  Monaco (parsed at submit); `prompt`/`modelId` apply to MODEL steps. */
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
  /** Free-param JSON-object text (parsed to bytes server-side). */
  readonly paramsText: string;
  /** Optional reasoning-mode (PR-4 Phase F) — opt-in MODEL knob; "" ⇒ default. */
  readonly reasoning: "" | "full" | "minimal" | "off";
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
    if (s.paramsText.trim() !== "" && !isJsonObject(s.paramsText)) {
      return `Step "${s.label}" params must be a JSON object.`;
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
    const params = paramsRecord(s);
    let prompt = s.prompt;
    if (s.kind === "model") {
      const instr = inbound.get(s.id);
      if (instr && instr.length > 0) {
        prompt = `${instr.join("\n\n")}\n\n${s.prompt}`.trim();
      }
    }
    const step: StepInput = {
      kind: s.kind,
      modelId: s.kind === "model" ? s.modelId : undefined,
      prompt: s.kind === "model" ? prompt : undefined,
      params,
    };
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

/** A fresh empty step of `kind` with a unique client-local id. */
export function newStep(kind: BuilderStepKind, id: string): BuilderStep {
  return {
    id,
    kind,
    label: kind === "model" ? "Agent" : "Step",
    modelId: "",
    prompt: "",
    paramsText: "",
    reasoning: "",
  };
}
