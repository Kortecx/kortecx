/**
 * The Chains DSL parity test — the TypeScript surface of the cross-surface
 * contract (`tests/golden/chains/SPEC.md` + `corpus.json`). Every golden case is
 * parsed + lowered; success cases must deep-equal `expect` (the snake_case shape),
 * error cases must raise the matching error class. Plus combinator-API tests
 * asserting parity with the string form (TS has no operator overloading).
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  Chain,
  ChainCycleError,
  ChainParseError,
  ChainUnknownHandleError,
  chain,
  chainFrom,
  group,
  par,
  seq,
  task,
} from "../src/chains.js";
import type { Lowered, Task } from "../src/chains.js";

// The golden corpus lives at the repo root; this test file is bindings/typescript/test.
const CORPUS_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "..",
  "tests",
  "golden",
  "chains",
  "corpus.json",
);

/** A task spec as it appears in the corpus (the resolved `tasks` map values). */
interface CorpusTask {
  kind: "pure" | "model" | "tool";
  model_id?: string;
  prompt?: string;
  params?: Record<string, string>;
  /** TOOL only: the single `{ tool_id: tool_version }` + the structured args. */
  tool_contract?: Record<string, string>;
  args?: Record<string, string | number | boolean>;
}

interface CorpusCase {
  name: string;
  dsl: string;
  seed: number;
  tasks: Record<string, CorpusTask>;
  expect?: Lowered;
  error?: "parse" | "unknown_handle" | "cycle";
}

/** Build the `Task` resolution map from a corpus case's `tasks` (the SDK's factory shape). */
function tasksFromCorpus(specs: Record<string, CorpusTask>): Record<string, Task> {
  const out: Record<string, Task> = {};
  for (const [handle, spec] of Object.entries(specs)) {
    if (spec.kind === "model") {
      out[handle] = task.model(spec.model_id ?? "", spec.prompt ?? "", spec.params ?? {});
    } else if (spec.kind === "tool") {
      const [toolId, version] = Object.entries(spec.tool_contract ?? {})[0] ?? ["", ""];
      out[handle] = task.tool(toolId, version, spec.args ?? {});
    } else {
      out[handle] = task.pure(spec.params ?? {});
    }
  }
  return out;
}

const corpus: CorpusCase[] = JSON.parse(readFileSync(CORPUS_PATH, "utf-8"));

describe("Chains DSL — golden corpus parity", () => {
  it("loads a non-empty corpus", () => {
    expect(corpus.length).toBeGreaterThan(0);
  });

  for (const c of corpus) {
    if (c.error !== undefined) {
      it(`${c.name}: rejects with the '${c.error}' error class`, () => {
        const run = () => chain(c.dsl, { tasks: tasksFromCorpus(c.tasks), seed: c.seed });
        const expectedClass =
          c.error === "parse"
            ? ChainParseError
            : c.error === "unknown_handle"
              ? ChainUnknownHandleError
              : ChainCycleError;
        expect(run).toThrow(expectedClass);
      });
    } else {
      it(`${c.name}: lowers to the canonical (steps, edges)`, () => {
        const lowered = chain(c.dsl, { tasks: tasksFromCorpus(c.tasks), seed: c.seed }).lower();
        expect(lowered).toEqual(c.expect);
      });
    }
  }
});

describe("Chains DSL — combinator API parity with the string form", () => {
  it("seq(a, b) == chain('a > b')", () => {
    const a = task.pure();
    const b = task.pure();
    const viaString = chain("a > b", { tasks: { a, b } }).lower();
    const viaCombinator = chainFrom(seq(a, b)).lower();
    expect(viaCombinator).toEqual(viaString);
  });

  it("seq(a, par(b, c)) == chain('a > [b & c]') (fan-out)", () => {
    const a = task.pure();
    const b = task.pure();
    const c = task.pure();
    const viaString = chain("a > [b & c]", { tasks: { a, b, c } }).lower();
    const viaCombinator = chainFrom(seq(a, par(b, c))).lower();
    expect(viaCombinator).toEqual(viaString);
  });

  it("seq(group(a, b), c) == chain('[a & b] > c') (fan-in)", () => {
    const a = task.pure();
    const b = task.pure();
    const c = task.pure();
    const viaString = chain("[a & b] > c", { tasks: { a, b, c } }).lower();
    const viaCombinator = chainFrom(seq(group(a, b), c)).lower();
    expect(viaCombinator).toEqual(viaString);
  });

  it("reuses the same Task object as the same node (bipartite join)", () => {
    const a = task.pure();
    const b = task.pure();
    const c = task.pure();
    const d = task.pure();
    const viaString = chain("[a & b] > [c & d]", { tasks: { a, b, c, d } }).lower();
    const viaCombinator = chainFrom(seq(par(a, b), par(c, d))).lower();
    expect(viaCombinator).toEqual(viaString);
    // Four distinct nodes, the full 2x2 bipartite edge set.
    expect(viaCombinator.steps).toHaveLength(4);
    expect(viaCombinator.edges).toHaveLength(4);
  });

  it("carries a model step's model_id + prompt through the combinator path", () => {
    const gen = task.model("kx-serve:qwen3-4b-q4_k_m", "Summarize the input.");
    const sum = task.pure({ label: "final" });
    const lowered = chainFrom(seq(gen, sum)).lower();
    expect(lowered.steps[0]).toEqual({
      kind: "model",
      model_id: "kx-serve:qwen3-4b-q4_k_m",
      prompt: "Summarize the input.",
      body_signature_id: null,
      tool_contract: {},
      params: {},
    });
    expect(lowered.steps[1]?.params).toEqual({ label: "final" });
  });

  it("the combinator path rejects a cycle (self-loop via reuse)", () => {
    const a = task.pure();
    expect(() => chainFrom(seq(a, a))).toThrow(ChainCycleError);
  });
});

describe("Chains DSL — build() feeds the BlueprintBuilder", () => {
  it("produces a FROZEN SubmitWorkflow init with the lowered topology + seed", () => {
    const a = task.pure({ topic: "hi" });
    const b = task.pure();
    const c = chain("a > b", { tasks: { a, b }, seed: 7 });
    const req = c.build();
    expect(req.seed).toBe(7);
    expect(req.steps).toHaveLength(2);
    expect(req.edges).toHaveLength(1);
    // The builder UTF-8-encodes params at build time.
    expect(req.steps?.[0]?.params?.topic).toEqual(new TextEncoder().encode("hi"));
    expect(c).toBeInstanceOf(Chain);
  });
});
