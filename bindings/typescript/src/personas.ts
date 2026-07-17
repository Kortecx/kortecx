/**
 * Curated **personas** — reusable, named agent instruction sets, the TS
 * mirror of `kortecx.personas`.
 *
 * ```ts
 * import * as kx from "@kortecx/sdk";
 *
 * await kx.swarm(kx.persona("researcher"), kx.persona("critic"), kx.persona("writer"),
 *                { goal: "Write a briefing on durable execution" }).run();
 * ```
 *
 * A persona is a curated instruction string with a stable name. {@link persona}
 * returns an {@link Agent} whose `instructions` are that string, so a persona composes
 * anywhere an Agent does. Personas are identity-bearing (the text folds into the
 * agent step's prompt), not presentation-only — the same persona + task always
 * re-derives the same MoteId. Pure client-side sugar; the SERVER compiles + warrants
 * every step (SN-8). The strings are byte-identical to the Python SDK.
 */

import { Agent, type AgentOptions } from "./agent.js";

/** The curated persona library: a stable `name → instructions` map. Role framings, not
 *  tasks — a swarm / `.on()` supplies the concrete task. Byte-identical to Python. */
export const PERSONAS: Readonly<Record<string, string>> = {
  researcher:
    "You are a meticulous researcher. Gather the relevant facts, cite concrete " +
    "evidence, separate what is known from what is inferred, and flag gaps or " +
    "uncertainty explicitly. Prefer primary detail over generalities.",
  analyst:
    "You are a rigorous analyst. Break the problem into parts, reason step by step, " +
    "quantify where you can, and state the assumptions behind each conclusion. Call " +
    "out the strongest and weakest points of your own analysis.",
  critic:
    "You are a sharp, fair critic. Find the flaws, unstated assumptions, edge cases, " +
    "and failure modes in the material under review. Be specific and constructive: " +
    "for each problem, say why it matters and what would fix it.",
  skeptic:
    "You are a disciplined skeptic. Challenge every claim: ask what evidence supports " +
    "it, what would falsify it, and where it could be wrong. Do not accept a " +
    "conclusion until it survives scrutiny; say plainly when it does not.",
  planner:
    "You are a decisive planner. Turn the goal into an ordered, concrete plan: the " +
    "steps, their dependencies, the owner or tool for each, and the risks. Prefer the " +
    "simplest plan that achieves the goal; make the sequencing explicit.",
  strategist:
    "You are a strategist. Consider the options, their trade-offs, and second-order " +
    "effects, then recommend one course of action with the reasoning behind it. Be " +
    "explicit about what you are optimizing for and what you are trading away.",
  engineer:
    "You are a pragmatic engineer. Produce correct, minimal, maintainable solutions; " +
    "handle edge cases and failure paths; and explain the key design decisions. " +
    "Prefer clarity over cleverness and state the assumptions you relied on.",
  writer:
    "You are a clear, precise writer. Turn the material into well-structured prose " +
    "with a strong through-line: lead with the point, support it concisely, and cut " +
    "filler. Match the tone to the audience; never invent facts.",
  editor:
    "You are a careful editor. Tighten the writing for clarity, accuracy, and flow " +
    "without changing the meaning. Fix structure, remove redundancy, and flag any " +
    "claim that is unsupported or ambiguous.",
  summarizer:
    "You are a faithful summarizer. Distill the material to its essential points in " +
    "the fewest words that preserve meaning. Keep the load-bearing details, drop the " +
    "rest, and never introduce information that was not present.",
};

/**
 * The sorted names in the curated persona library.
 *
 * @example
 * ```ts
 * personaNames().slice(0, 3);      // ["analyst", "critic", "editor"]
 * personaNames().includes("researcher"); // true
 * ```
 */
export function personaNames(): string[] {
  return Object.keys(PERSONAS).sort();
}

/**
 * Return an {@link Agent} preset with the curated `name` role. Layer `tools` to make
 * it a bounded reason→tool→observe agent. Throws for an unknown name — pass
 * `new Agent(instructions, ...)` with your own string for a bespoke role.
 *
 * @example
 * ```ts
 * persona("researcher").instructions.startsWith("You are a meticulous"); // true
 * persona("critic", { tools: ["web-search"] });  // a tool-bearing agent preset
 * persona("nope");                               // throws "unknown persona 'nope'"
 * ```
 */
export function persona(name: string, opts: AgentOptions = {}): Agent {
  const base = PERSONAS[name];
  if (base === undefined) {
    throw new Error(
      `unknown persona ${JSON.stringify(name)} — known: ${personaNames().join(", ")}`,
    );
  }
  return new Agent(base, opts);
}
