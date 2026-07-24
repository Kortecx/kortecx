/**
 * POC-5d: the pure round-trip adapter between an App's portable blueprint (a
 * {@link DagSpecJson} carried verbatim in the `kortecx.app/v1` envelope) and the
 * visual builder's {@link BuilderGraph} — so the single-App Lineage editor REUSES
 * the BlueprintBuilder canvas. No React, no reactflow — exhaustively unit-testable
 * (mirrors `builder-graph.ts` / the Rust core's pure/total/testable discipline).
 *
 * The load-bearing CORRECTNESS property (the digest-safety guarantee, proven in
 * `app-blueprint.test.ts`): for any round-trippable blueprint `bp`,
 *   `Chain.fromBlueprint(builderGraphToBlueprint(appBlueprintToBuilderGraph(bp).graph, …))`
 * compiles BYTE-IDENTICAL to `Chain.fromBlueprint(bp)` — i.e. a lineage edit never
 * changes what the blueprint COMPILES to beyond the user's intended edit.
 *
 * Lossless-ness (defense in depth — a save must NEVER silently corrupt a
 * model-authored blueprint): the Lineage editor only ever replaces `envelope.blueprint`
 * (the caller spreads `{...envelope, blueprint}`); everything OUTSIDE the blueprint
 * (references / steering_config / replay / input_schema / …) is carried verbatim.
 * Inside the blueprint, fields BuilderGraph does not model are PRESERVED:
 *  - blueprint-level `seed` / `execution_mode` / `context_bundles` (via the snapshot
 *    {@link UnmodeledReport} passed back into {@link builderGraphToBlueprint});
 *  - per-step `body_signature_id` (an `exec` step) ⇒ `refuseEdit` (the canvas goes
 *    READ-ONLY — an un-round-trippable blueprint can never be saved);
 *  - per-edge `non_cascade` (re-merged by source→target).
 */

import type { DagSpecJson, DagSpecStep } from "@kortecx/sdk/web";
import {
  APP_SCHEMA,
  REACT_MAX_TOOL_CALLS_KEY,
  REACT_MAX_TURNS_KEY,
  TOOL_ARGS_KEY,
} from "@kortecx/sdk/web";
import type { BuilderEdge, BuilderGraph, BuilderStep, BuilderStepKind } from "./builder-graph";

/** What the editor must preserve / refuse from a parsed blueprint. */
export interface UnmodeledReport {
  /** `true` ⇒ the blueprint has a structure the visual editor cannot faithfully
   *  round-trip (an `exec` / `body_signature_id` step) — the canvas is READ-ONLY and
   *  Save is hidden (never a lossy save, GR15). */
  readonly refuseEdit: boolean;
  /** A human reason when `refuseEdit` is true. */
  readonly reason: string | null;
  /** Blueprint-level fields BuilderGraph does not carry — preserved on Save. */
  readonly seed: number;
  readonly executionMode: string | undefined;
  readonly contextBundles: readonly string[];
  /** Per-edge `non_cascade`, keyed `${parentIndex}>${childIndex}`. */
  readonly edgeNonCascade: ReadonlyMap<string, boolean>;
}

/** Mirror the CLI `StepSpec::resolve_kind` / SDK `inferKind` (model fields ⇒ model;
 *  a tool contract ⇒ tool; else pure). An explicit `pure|model|tool` wins; `exec`
 *  (or a `body_signature_id`) is unrepresentable → handled by the caller as refuse. */
function builderKind(d: DagSpecStep): BuilderStepKind {
  if (d.kind === "model" || d.kind === "tool" || d.kind === "pure") {
    return d.kind;
  }
  if (d.model_id || d.prompt) {
    return "model";
  }
  if (d.tool_contract && Object.keys(d.tool_contract).length > 0) {
    return "tool";
  }
  return "pure";
}

function isUnrepresentable(d: DagSpecStep): boolean {
  return d.kind === "exec" || (d.body_signature_id != null && d.body_signature_id !== "");
}

function reasoningOf(value: string | undefined): BuilderStep["reasoning"] {
  return value === "full" || value === "minimal" || value === "off" ? value : "";
}

/** Pretty-print a non-empty record as the Monaco `paramsText`, else "". */
function jsonText(obj: Record<string, unknown>): string {
  return Object.keys(obj).length > 0 ? JSON.stringify(obj, null, 2) : "";
}

/** A copy of `obj` without `keys` — the modeled fields (budget / reasoning / tool
 *  args) are stripped from the editable free-params so they never double-emit. */
function stripKeys(obj: Record<string, string>, keys: readonly string[]): Record<string, string> {
  const drop = new Set(keys);
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(obj)) {
    if (!drop.has(k)) {
      out[k] = v;
    }
  }
  return out;
}

/**
 * Parse a blueprint's `steps`/`edges` into a {@link BuilderGraph} the canvas can
 * render + edit, plus an {@link UnmodeledReport} (preserve/refuse). Step ids are the
 * ordinal `s${i}` (the array index IS the identity, matching the seed convention).
 */
export function appBlueprintToBuilderGraph(blueprint: DagSpecJson): {
  graph: BuilderGraph;
  unmodeled: UnmodeledReport;
} {
  let refuseEdit = false;
  let reason: string | null = null;

  const steps: BuilderStep[] = (blueprint.steps ?? []).map((d, i): BuilderStep => {
    if (isUnrepresentable(d)) {
      refuseEdit = true;
      reason =
        "This App's blueprint has an exec/binary step the visual editor can't safely edit. View it read-only or edit it via the SDK/CLI.";
    }
    const kind = builderKind(d);
    const rawParams: Record<string, string> = { ...(d.params ?? {}) };

    // Budget: prefer the top-level CLI form, else the folded `params` form.
    let maxTurns: number | undefined;
    let maxToolCalls: number | undefined;
    if (d.max_turns !== undefined) {
      maxTurns = d.max_turns;
    } else if (rawParams[REACT_MAX_TURNS_KEY] !== undefined) {
      maxTurns = Number(rawParams[REACT_MAX_TURNS_KEY]);
    }
    if (d.max_tool_calls !== undefined) {
      maxToolCalls = d.max_tool_calls;
    } else if (rawParams[REACT_MAX_TOOL_CALLS_KEY] !== undefined) {
      maxToolCalls = Number(rawParams[REACT_MAX_TOOL_CALLS_KEY]);
    }

    // Reasoning chip (an opt-in declared free-param).
    const reasoning = reasoningOf(rawParams.reasoning);
    // The editable free params exclude every field other UI controls model.
    const params = stripKeys(rawParams, [
      REACT_MAX_TURNS_KEY,
      REACT_MAX_TOOL_CALLS_KEY,
      TOOL_ARGS_KEY,
      ...(reasoning !== "" ? ["reasoning"] : []),
    ]);

    const toolContract: Record<string, string> = { ...(d.tool_contract ?? {}) };

    if (kind === "tool") {
      // A TOOL step: its single tool_contract entry is the tool id@version; the
      // call args (CLI `args` map OR the folded `kx.tool.args` blob) become the
      // editable `paramsText` (builder-graph lowers `paramsText` back to the args).
      const [toolId, toolVersion] = Object.entries(toolContract)[0] ?? ["", "1"];
      const args = d.args ?? parseFoldedArgs(rawParams[TOOL_ARGS_KEY]);
      return {
        id: `s${i}`,
        kind,
        label: "Tool",
        modelId: "",
        prompt: "",
        paramsText: jsonText(args),
        reasoning: "",
        toolId,
        toolVersion: toolVersion || "1",
        toolContract: {},
        // A tool step runs no agentic loop and reads no instructions, so it can carry no
        // App capability binding — `RunApp` drops one there with a warning either way.
        skills: [],
        connections: [],
        datasets: [],
        apps: [],
        maxTurns: undefined,
        maxToolCalls: undefined,
      };
    }

    return {
      id: `s${i}`,
      kind,
      label: kind === "model" ? "Agent" : "Step",
      modelId: d.model_id ?? "",
      prompt: d.prompt ?? "",
      paramsText: jsonText(params),
      reasoning,
      toolId: "",
      toolVersion: "",
      // An agentic model step keeps its tool_contract (the bounded ReAct set) and its
      // per-node App capability bindings.
      toolContract: kind === "model" ? toolContract : {},
      skills: kind === "model" ? [...(d.skills ?? [])] : [],
      connections: kind === "model" ? [...(d.connections ?? [])] : [],
      datasets: kind === "model" ? [...(d.datasets ?? [])] : [],
      apps: kind === "model" ? [...(d.apps ?? [])] : [],
      maxTurns: kind === "model" ? maxTurns : undefined,
      maxToolCalls: kind === "model" ? maxToolCalls : undefined,
    };
  });

  const edgeNonCascade = new Map<string, boolean>();
  const edges: BuilderEdge[] = (blueprint.edges ?? []).map((e, i): BuilderEdge => {
    if (e.non_cascade) {
      edgeNonCascade.set(`${e.parent}>${e.child}`, true);
    }
    return {
      id: `e${i}-s${e.parent}-s${e.child}`,
      source: `s${e.parent}`,
      target: `s${e.child}`,
      edge: e.edge === "control" ? "control" : "data",
      // Edge instructions are a run-only fold (no DagSpec representation); the
      // Lineage editor hides the field, so it round-trips as "".
      instruction: "",
    };
  });

  return {
    graph: { steps, edges },
    unmodeled: {
      refuseEdit,
      reason,
      seed: blueprint.seed ?? 0,
      executionMode: blueprint.execution_mode,
      contextBundles: blueprint.context_bundles ?? [],
      edgeNonCascade,
    },
  };
}

/**
 * Serialize an edited {@link BuilderGraph} back to a {@link DagSpecJson}, re-merging
 * the {@link UnmodeledReport}'s preserved blueprint-level fields. Emits the
 * self-describing explicit-`kind`, CLI args-separated form (a tool step's `args` map;
 * an agentic step's top-level `max_turns`/`max_tool_calls`) — which
 * `Chain.fromBlueprint` imports identically to the SDK folded form, so the round-trip
 * is compile-equivalent (the digest-safety property). The `id` ordinal `s${n}` IS the
 * step index. NEVER call this on a `refuseEdit` graph (the canvas is read-only there).
 */
export function builderGraphToBlueprint(
  graph: BuilderGraph,
  unmodeled: Pick<UnmodeledReport, "seed" | "executionMode" | "contextBundles" | "edgeNonCascade">,
): DagSpecJson {
  const indexOf = new Map<string, number>();
  graph.steps.forEach((s, i) => indexOf.set(s.id, i));

  const steps: DagSpecStep[] = graph.steps.map((s): DagSpecStep => {
    const step: DagSpecStep = { kind: s.kind };
    const params = parseParams(s.paramsText);
    if (s.reasoning !== "" && s.kind === "model") {
      params.reasoning = s.reasoning;
    }

    if (s.kind === "tool") {
      const version = s.toolVersion.trim() === "" ? "1" : s.toolVersion;
      step.tool_contract = { [s.toolId]: version };
      const args = parseParams(s.paramsText);
      if (Object.keys(args).length > 0) {
        step.args = args;
      }
      return step;
    }

    if (s.kind === "model") {
      if (s.modelId) {
        step.model_id = s.modelId;
      }
      if (s.prompt) {
        step.prompt = s.prompt;
      }
      if (Object.keys(s.toolContract).length > 0) {
        step.tool_contract = { ...s.toolContract };
        if (s.maxTurns !== undefined) {
          step.max_turns = s.maxTurns;
        }
        if (s.maxToolCalls !== undefined) {
          step.max_tool_calls = s.maxToolCalls;
        }
      }
      // The per-node App capability bindings. Emitted ONLY when non-empty, so a graph
      // that binds nothing produces the same blueprint bytes it always did — which is what
      // keeps an existing App's identity unchanged when it is re-saved from the canvas.
      if (s.skills.length > 0) {
        step.skills = [...s.skills];
      }
      if (s.connections.length > 0) {
        step.connections = [...s.connections];
      }
      if (s.datasets.length > 0) {
        step.datasets = [...s.datasets];
      }
      if (s.apps.length > 0) {
        step.apps = [...s.apps];
      }
    }
    if (Object.keys(params).length > 0) {
      step.params = params;
    }
    return step;
  });

  const blueprint: DagSpecJson = { seed: unmodeled.seed, steps };
  if (unmodeled.executionMode) {
    blueprint.execution_mode = unmodeled.executionMode;
  }
  const edges = graph.edges
    .map((e) => {
      const parent = indexOf.get(e.source);
      const child = indexOf.get(e.target);
      if (parent === undefined || child === undefined) {
        return null; // dangling — dropped (a deleted node drops its edges)
      }
      const out: { parent: number; child: number; edge?: string; non_cascade?: boolean } = {
        parent,
        child,
      };
      if (e.edge === "control") {
        out.edge = "control";
      }
      if (unmodeled.edgeNonCascade.get(`${parent}>${child}`)) {
        out.non_cascade = true;
      }
      return out;
    })
    .filter((e): e is NonNullable<typeof e> => e !== null);
  if (edges.length > 0) {
    blueprint.edges = edges;
  }
  if (unmodeled.contextBundles.length > 0) {
    blueprint.context_bundles = [...unmodeled.contextBundles];
  }
  return blueprint;
}

/** Parse a JSON-object `paramsText` to a string-valued record (blank ⇒ {}). The
 *  builder already validates it is a JSON object before Save; values are coerced to
 *  strings the way `builder-graph.ts` does. */
function parseParams(text: string): Record<string, string> {
  if (text.trim() === "") {
    return {};
  }
  try {
    const parsed = JSON.parse(text) as Record<string, unknown>;
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed)) {
      out[k] = typeof v === "string" ? v : JSON.stringify(v);
    }
    return out;
  } catch {
    return {};
  }
}

/** The unmodeled snapshot for a BRAND-NEW App blueprint — no preserved fields (seed 0,
 *  no exec / execution-mode / context-bundles / non-cascade edges). Read-only at use. */
export const FRESH_UNMODELED: Pick<
  UnmodeledReport,
  "seed" | "executionMode" | "contextBundles" | "edgeNonCascade"
> = {
  seed: 0,
  executionMode: undefined,
  contextBundles: [],
  edgeNonCascade: new Map<string, boolean>(),
};

/**
 * Save-to-App (POC-5d): replace ONLY `envelope.blueprint` from the edited graph. Every
 * other envelope region — `references` / `steering_config` / `input_schema` / `tags` /
 * `replay` — rides VERBATIM (the lossless rule; a structure edit never corrupts a
 * model-authored envelope beyond the intended blueprint change). NEVER call on a
 * `refuseEdit` graph.
 */
export function structureSaveEnvelope(
  envelope: Record<string, unknown>,
  graph: BuilderGraph,
  unmodeled: Pick<UnmodeledReport, "seed" | "executionMode" | "contextBundles" | "edgeNonCascade">,
): Record<string, unknown> {
  return { ...envelope, blueprint: builderGraphToBlueprint(graph, unmodeled) };
}

/**
 * Save-as-App: mint a fresh MINIMAL `kortecx.app/v1` envelope from a graph, matching the
 * SDK `AppBuilder.toEnvelope` minimal shape (schema + name + version + blueprint). The
 * capability rails (tools / connections / skills) are authored afterwards on the App page.
 */
export function newAppEnvelope(name: string, graph: BuilderGraph): Record<string, unknown> {
  return {
    schema: APP_SCHEMA,
    name,
    version: "1",
    blueprint: builderGraphToBlueprint(graph, FRESH_UNMODELED),
  };
}

/** Parse a folded `kx.tool.args` JSON blob back to an args record (best-effort). */
function parseFoldedArgs(blob: string | undefined): Record<string, string> {
  if (blob === undefined || blob.trim() === "") {
    return {};
  }
  try {
    const parsed = JSON.parse(blob) as Record<string, unknown>;
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed)) {
      out[k] = typeof v === "string" ? v : JSON.stringify(v);
    }
    return out;
  } catch {
    return {};
  }
}
