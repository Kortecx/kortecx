/**
 * The App Lineage diagram's PURE per-step view-model (no React) — what each node card
 * shows, derived from the parsed {@link BuilderGraph} + the stored App envelope.
 *
 * Why a derived title: the portable blueprint has NO per-step label. `DagSpecStep`
 * (SDK `chains.ts`) carries no `label`/`name`/`id`, so `appBlueprintToBuilderGraph`
 * hard-codes "Agent"/"Tool"/"Step" — an 8-step App renders 8 identical cards. The
 * title is therefore DERIVED from what the step actually carries (prompt → model →
 * tool → ordinal), which is the difference between a diagram that shows topology and
 * one that shows the App.
 *
 * HONESTY (SN-8) — the rules this module will not break:
 *  - A `tool_contract` is a WISH, not a grant. The server intersects it against the
 *    caller's authority ∩ fireable ∩ registry ∩ compat at run. Card language is
 *    "requests", never "has".
 *  - A budget is rendered ONLY when the blueprint explicitly carries it. The default
 *    is genuinely contested in-tree (`builder-graph.ts` documents "default 20,
 *    ceiling 20" but validates with `?? 6` under `calls < turns <= 8`; the CLI
 *    declares `DEFAULT_MAX_TOOL_CALLS = 20`), so asserting one would state a number
 *    the codebase itself does not agree on.
 *  - An empty `model_id` is not "no model" — the server binds the served model (or the
 *    App's `model_route`) at run. Say that, rather than showing a blank.
 */

import type { BuilderGraph, BuilderStep } from "../builder/builder-graph";

/** Max characters of a derived title before ellipsis (the card is one line wide). */
const TITLE_MAX = 48;

/** How many tool chips a card renders before collapsing the rest into "+N". */
export const TOOL_CHIP_MAX = 2;

/** One tool the step REQUESTS (id + version) — never a granted capability. */
export interface ToolChip {
  readonly id: string;
  readonly version: string;
}

/** The rendered shape of one lineage node. Every field beyond `ordinal`/`kind`/`title`
 *  is optional: a blueprint step may carry almost nothing, and the card degrades
 *  rather than inventing content. */
export interface LineageStepView {
  readonly id: string;
  /** 1-based step number. The blueprint has no step id — the ARRAY INDEX is the
   *  identity (it is what `edges[].parent/child` reference), so the ordinal is the
   *  honest, stable thing to show. */
  readonly ordinal: number;
  readonly kind: BuilderStep["kind"];
  /** The derived title (never blank — falls back to `Step N`). */
  readonly title: string;
  /** The full prompt, for a native `title` tooltip ("" when the step has none). */
  readonly tooltip: string;
  /** The model line: an explicit id, or how the server will bind one at run. */
  readonly model: string | null;
  /** True when `model` describes a run-time binding rather than an authored id
   *  (rendered muted — it is a deferral, not a fact about this step). */
  readonly modelInferred: boolean;
  /** The tools this step REQUESTS (a TOOL step's single tool, or a MODEL step's
   *  bounded ReAct contract), capped at {@link TOOL_CHIP_MAX}. */
  readonly tools: readonly ToolChip[];
  /** How many requested tools are not shown as chips (0 ⇒ none hidden). */
  readonly toolsOverflow: number;
  /** The agentic budget, ONLY when explicitly authored (never a synthesized default). */
  readonly budget: string | null;
  /** The opt-in reasoning mode ("" ⇒ unset ⇒ not rendered). */
  readonly reasoning: string;
  /** True on the step the server folds the App's skills + tool wishes onto. */
  readonly isEntry: boolean;
}

/**
 * The first line of `text` with real content, trimmed + ellipsized. A prompt is often
 * a multi-line instruction whose first line reads as its purpose; anything past that
 * is detail the tooltip carries.
 */
function firstLine(text: string, max = TITLE_MAX): string {
  const line = text
    .split("\n")
    .map((l) => l.trim())
    .find((l) => l.length > 0);
  if (line === undefined) {
    return "";
  }
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

/** A step's tool wishes as chips: a TOOL step fires its one registered tool; a MODEL
 *  step may carry a bounded ReAct contract. A PURE step requests nothing. */
function toolChips(step: BuilderStep): ToolChip[] {
  if (step.kind === "tool") {
    return step.toolId === "" ? [] : [{ id: step.toolId, version: step.toolVersion || "1" }];
  }
  if (step.kind === "model") {
    return Object.entries(step.toolContract).map(([id, version]) => ({ id, version }));
  }
  return [];
}

/**
 * The model line. An empty `model_id` is the portable-App convention (POC-5d): the
 * SERVER binds the model at run, so we say which binding applies rather than showing a
 * blank or inventing an id. `modelRoute` is the App's `steering_config.model.model_route`.
 */
function modelLine(
  step: BuilderStep,
  modelRoute: string,
): { model: string | null; modelInferred: boolean } {
  if (step.kind !== "model") {
    return { model: null, modelInferred: false };
  }
  if (step.modelId !== "") {
    return { model: step.modelId, modelInferred: false };
  }
  return modelRoute !== ""
    ? { model: `inherits ${modelRoute}`, modelInferred: true }
    : { model: "served model at run", modelInferred: true };
}

/**
 * The agentic budget line — ONLY from explicitly authored values. `maxTurns`/
 * `maxToolCalls` are `undefined` when the blueprint omits them, and this renders
 * nothing rather than guessing a default (see the module header).
 */
function budgetLine(step: BuilderStep): string | null {
  if (step.kind !== "model") {
    return null;
  }
  const parts: string[] = [];
  if (step.maxTurns !== undefined) {
    parts.push(`${step.maxTurns} turns`);
  }
  if (step.maxToolCalls !== undefined) {
    parts.push(`${step.maxToolCalls} calls`);
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

/**
 * The ENTRY AGENTIC step — the first MODEL step that is a DAG ROOT (no incoming edge),
 * or `null` when there is none.
 *
 * A DELIBERATE MIRROR of the server's `entry_agentic_step_index`
 * (`crates/kx-gateway/src/app_run.rs`), which is the AUTHORITY. That function is a pure
 * function of the blueprint's steps + edges, so the console can compute the same answer
 * rather than guess. It is exactly where `RunApp` folds the App's skill instructions AND
 * the combined tool wish, and the two must co-locate: a `pure → model` chain gets NO
 * fold (the instructions would land on the pure root the model never reads — the server
 * refuses that split), which is why "first model step" alone is the wrong rule.
 *
 * Kind parity: the server's `is_model_step` is `kind == "model"`, else `model_id ||
 * prompt` non-empty — which is what `builderKind` already resolved into `step.kind`.
 * (A contradictory step, e.g. `kind:"pure"` beside a `model_id`, is rejected outright by
 * the server's `resolve_kind`, so it cannot reach a runnable App.)
 *
 * The unit tests mirror the Rust test table 1:1 so this cannot silently drift.
 */
export function entryAgenticStepId(graph: BuilderGraph): string | null {
  const hasIncoming = new Set(graph.edges.map((e) => e.target));
  const entry = graph.steps.find((s) => s.kind === "model" && !hasIncoming.has(s.id));
  return entry?.id ?? null;
}

/**
 * The warning shown when an App declares skills / tool grants but its blueprint has NO
 * root model step to fold them onto. The server drops them at run with only a
 * `tracing::warn!` — invisible to the user, who sees a populated Skills rail and
 * reasonably assumes it applies. This is the one place that silent drop becomes visible.
 * `null` ⇒ nothing to warn about.
 */
export function entryFoldWarning(graph: BuilderGraph, wishes: number): string | null {
  if (wishes === 0 || entryAgenticStepId(graph) !== null) {
    return null;
  }
  const [what, it] =
    wishes === 1 ? ["1 skill/tool wish", "it"] : [`${wishes} skill/tool wishes`, "them"];
  return `${what} can't be applied: this App's structure has no root agent step to fold ${it} onto, so the run drops ${it}.`;
}

/**
 * The diagram's accessible name. `role="img"` collapses the whole subtree, so every card's
 * detail is unreachable to a screen reader unless the label carries it — the richer the
 * cards get, the more this matters. One sentence per step, in blueprint order.
 */
export function diagramLabel(views: readonly LineageStepView[]): string {
  if (views.length === 0) {
    return "App structure diagram: no steps.";
  }
  const steps = views.map((v) => {
    const bits = [`Step ${v.ordinal}, ${v.kind}: ${v.title}`];
    if (v.model !== null) {
      bits.push(`model ${v.model}`);
    }
    if (v.tools.length > 0) {
      const names = v.tools.map((t) => t.id).join(", ");
      bits.push(
        v.toolsOverflow > 0 ? `requests ${names} and ${v.toolsOverflow} more` : `requests ${names}`,
      );
    }
    if (v.isEntry) {
      bits.push("the entry step the App's skills and tool wishes fold onto");
    }
    return bits.join("; ");
  });
  const n = views.length === 1 ? "1 step" : `${views.length} steps`;
  return `App structure diagram, ${n}. ${steps.join(". ")}.`;
}

/** Build the per-step view-models for a parsed blueprint. Pure + total. */
export function lineageStepViews(graph: BuilderGraph, modelRoute: string): LineageStepView[] {
  const entryId = entryAgenticStepId(graph);
  return graph.steps.map((step, i) => {
    const chips = toolChips(step);
    const { model, modelInferred } = modelLine(step, modelRoute);
    return {
      id: step.id,
      ordinal: i + 1,
      kind: step.kind,
      title: deriveTitle(step, i),
      tooltip: step.prompt,
      model,
      modelInferred,
      tools: chips.slice(0, TOOL_CHIP_MAX),
      toolsOverflow: Math.max(0, chips.length - TOOL_CHIP_MAX),
      budget: budgetLine(step),
      reasoning: step.reasoning,
      isEntry: step.id === entryId,
    };
  });
}

/**
 * A step's display title, in descending order of how much it tells the reader:
 * its prompt's opening line → its model → its tool → its position. The last rung
 * always succeeds, so a title is never blank.
 */
function deriveTitle(step: BuilderStep, index: number): string {
  const fromPrompt = firstLine(step.prompt);
  if (fromPrompt !== "") {
    return fromPrompt;
  }
  if (step.kind === "model" && step.modelId !== "") {
    return step.modelId;
  }
  if (step.kind === "tool" && step.toolId !== "") {
    return `${step.toolId}@${step.toolVersion || "1"}`;
  }
  return `Step ${index + 1}`;
}
