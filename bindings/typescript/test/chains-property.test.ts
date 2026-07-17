/**
 * Property / fuzz tests for the Chains string-DSL parser + lowering (SN-4 v2 #5).
 *
 * The Chains DSL accepts user-supplied text, so it gets round-trip + robustness
 * invariants asserted across the input space — not just the golden corpus cases in
 * `chains.test.ts`. Covered (mirrors the Python `test_chains_property.py`):
 *
 * - **precedence & edge generation** — `>` (seq, tightest) binds tighter than `&`
 *   binds tighter than `|`; over a flat chain of DISTINCT handles the DATA-edge set is
 *   exactly the adjacent `>` pairs (an INDEPENDENT oracle, not the parser's own output).
 * - **`@`-grants** — order-preserving dedup into a MODEL step's tool contract (version
 *   `"1"`); a `@`-grant on a non-model handle is a fail-closed `ChainAgenticError`.
 * - **handle dedup** — reusing a handle is ONE node.
 * - **cycle detection** — a handle-reuse that closes a `>` loop raises `ChainCycleError`.
 * - **fuzz robustness** — arbitrary text lowers or throws a declared `Chain*Error`,
 *   never an undeclared/generic error.
 * - **blueprint round-trip** — `fromBlueprint(toBlueprint())` re-compiles to the same
 *   `SubmitWorkflowRequest` init shape as `build()`.
 *
 * These need no server (the DSL describes topology; the SERVER compiles + warrants).
 */

import fc from "fast-check";
import { describe, expect, it } from "vitest";
import {
  Chain,
  ChainAgenticError,
  ChainCycleError,
  ChainParseError,
  ChainUnknownHandleError,
  chain,
  task,
} from "../src/chains.js";
import type { Task } from "../src/chains.js";

const OPERATORS = [">", "&", "|"] as const;
type Operator = (typeof OPERATORS)[number];

const distinctHandles = (n: number): string[] => Array.from({ length: n }, (_, i) => `h${i}`);

/** `h0 OP0 h1 OP1 h2 …` — a flat binary chain of distinct handles. */
function flatExpr(handles: string[], ops: Operator[]): string {
  let expr = handles[0] ?? "";
  ops.forEach((op, i) => {
    expr += ` ${op} ${handles[i + 1]}`;
  });
  return expr;
}

const pureTasks = (handles: string[]): Record<string, Task> =>
  Object.fromEntries(handles.map((h) => [h, task.pure()]));

describe("chains DSL — properties", () => {
  it("precedence: lowered edges are exactly the adjacent `>` pairs (independent oracle)", () => {
    fc.assert(
      fc.property(
        fc.array(fc.constantFrom(...OPERATORS), { minLength: 1, maxLength: 7 }),
        (ops) => {
          const n = ops.length + 1;
          const handles = distinctHandles(n);
          const low = chain(flatExpr(handles, ops), { tasks: pureTasks(handles) }).lower();

          expect(low.steps.map((s) => s.kind)).toEqual(Array(n).fill("pure"));
          const expected = ops
            .map((op, i) => (op === ">" ? ([i, i + 1] as const) : null))
            .filter((x): x is readonly [number, number] => x !== null);
          const got = low.edges.map((e) => [e.parent, e.child] as const);
          expect(got).toEqual(expected); // already canonically sorted + deduped
        },
      ),
    );
  });

  it("bracket grouping matches operator precedence (`>` > `&` > `|`)", () => {
    const order: Record<Operator, number> = { ">": 0, "&": 1, "|": 2 };
    fc.assert(
      fc.property(fc.constantFrom(...OPERATORS), fc.constantFrom(...OPERATORS), (left, right) => {
        const tasks = pureTasks(["a", "b", "c"]);
        const flat = chain(`a ${left} b ${right} c`, { tasks }).lower();
        const grouped =
          order[left] <= order[right]
            ? chain(`[a ${left} b] ${right} c`, { tasks }).lower()
            : chain(`a ${left} [b ${right} c]`, { tasks }).lower();
        expect(flat).toEqual(grouped);
      }),
    );
  });

  it("`@`-grants fold into a MODEL step as an order-preserving dedup at version '1'", () => {
    fc.assert(
      fc.property(fc.array(fc.constantFrom("t0", "t1", "t2", "t3"), { maxLength: 8 }), (tags) => {
        const expr = `m${tags.map((t) => `@${t}`).join("")}`;
        const low = chain(expr, { tasks: { m: task.model("", "go") } }).lower();
        const contract = low.steps[0]?.tool_contract ?? {};
        expect(Object.keys(contract)).toEqual([...new Set(tags)]);
        expect(Object.values(contract).every((v) => v === "1")).toBe(true);
      }),
    );
  });

  it("a `@`-grant on a non-model handle is a fail-closed ChainAgenticError", () => {
    fc.assert(
      fc.property(fc.constantFrom("t0", "t1", "t2"), (tag) => {
        expect(() => chain(`p@${tag}`, { tasks: { p: task.pure() } }).lower()).toThrow(
          ChainAgenticError,
        );
      }),
    );
  });

  it("handle reuse is one node: `a & a & … & a` lowers to 1 node, 0 edges", () => {
    fc.assert(
      fc.property(fc.integer({ min: 1, max: 8 }), (count) => {
        const expr = Array(count).fill("a").join(" & ");
        const low = chain(expr, { tasks: { a: task.pure() } }).lower();
        expect(low.steps.length).toBe(1);
        expect(low.edges).toEqual([]);
      }),
    );
  });

  it("a parallel merge of a handle multiset has exactly `distinct` nodes, 0 edges", () => {
    fc.assert(
      fc.property(fc.integer({ min: 2, max: 6 }), fc.integer({ min: 0, max: 5 }), (n, reuse) => {
        const handles = distinctHandles(n);
        const multiset = [...handles, ...handles.slice(0, reuse % (n + 1))];
        const low = chain(multiset.join(" & "), { tasks: pureTasks(handles) }).lower();
        expect(low.steps.length).toBe(new Set(multiset).size);
        expect(low.edges).toEqual([]);
      }),
    );
  });

  it("closing a `>` loop via handle reuse raises ChainCycleError", () => {
    fc.assert(
      fc.property(fc.integer({ min: 1, max: 6 }), (n) => {
        const handles = distinctHandles(n);
        const tasks = Object.fromEntries(handles.map((h) => [h, task.model("", "x")]));
        const expr = [...handles, handles[0]].join(" > ");
        expect(() => chain(expr, { tasks }).lower()).toThrow(ChainCycleError);
      }),
    );
  });

  it("fuzz: arbitrary text lowers or throws a declared Chain*Error — never an undeclared error", () => {
    const alphabet = "abcdh0123_-@&|[]> \t.$".split("");
    fc.assert(
      fc.property(fc.stringOf(fc.constantFrom(...alphabet), { maxLength: 48 }), (text) => {
        const tasks: Record<string, Task> = {
          a: task.pure(),
          b: task.model("", "x"),
          c: task.pure(),
        };
        try {
          chain(text, { tasks }).lower();
        } catch (err) {
          const declared =
            err instanceof ChainParseError ||
            err instanceof ChainUnknownHandleError ||
            err instanceof ChainCycleError ||
            err instanceof ChainAgenticError;
          expect(declared).toBe(true);
        }
      }),
    );
  });

  it("blueprint round-trip: fromBlueprint(toBlueprint()) == build()", () => {
    fc.assert(
      fc.property(fc.array(fc.constantFrom(...OPERATORS), { maxLength: 5 }), (ops) => {
        const n = ops.length + 1;
        const handles = distinctHandles(n);
        const tasks: Record<string, Task> = Object.fromEntries(
          handles.map((h, i) => [h, i % 2 === 0 ? task.model("", `p${i}`) : task.pure()]),
        );
        const c = chain(flatExpr(handles, ops), { tasks });
        expect(Chain.fromBlueprint(c.toBlueprint())).toEqual(c.build());
      }),
    );
  });
});
