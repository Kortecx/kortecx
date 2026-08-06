/**
 * Recovering a run's anchors from its own turn record — what makes a run started from
 * the CLI openable as itself rather than as "everything this server ever ran".
 */

import { describe, expect, it } from "vitest";
import { chainAnchors, chainContaining } from "../../src/lib/react-chain-anchors";
import { mergeServerRuns } from "../../src/lib/recent-runs";

const SALT_A = "aa".repeat(32);
const SALT_B = "bb".repeat(32);
const INSTANCE = "11".repeat(16);
let seq = 0;
const row = (turn: number, stepSalt: string, turnMoteId: string) => ({
  turn,
  stepSalt,
  turnMoteId,
  // A turn carries no clock; `seq` is the only cross-chain ordering there is, and
  // segmenting unsalted chains depends on it. Every real row has one.
  seq: seq++,
});

describe("chainAnchors", () => {
  it("groups rows by chain key — one entry per agentic submission", () => {
    // Two `kx agent run` invocations against one serve share an instance id, so the
    // chain key is the only thing that separates them.
    const out = chainAnchors([
      row(1, SALT_B, "b1"),
      row(0, SALT_B, "b0"),
      row(1, SALT_A, "a1"),
      row(0, SALT_A, "a0"),
    ]);
    expect(out.map((a) => a.stepSalt)).toEqual([SALT_B, SALT_A]);
    expect(out.map((a) => a.turns)).toEqual([2, 2]);
  });

  it("a group's key is the ?chain= and its TURN-0 Mote is the ?terminal=", () => {
    // Turn 0 is the admitted Mote, and the only anchor that resolves when the chain key
    // is a seed the coordinator discarded.
    const out = chainAnchors([row(2, SALT_A, "a2"), row(0, SALT_A, "a0"), row(1, SALT_A, "a1")]);
    expect(out).toEqual([
      { stepSalt: SALT_A, turn0MoteId: "a0", turnMoteIds: ["a0", "a1", "a2"], turns: 3 },
    ]);
  });

  it("finds turn 0 whatever order the page arrives in", () => {
    const ascending = chainAnchors([row(0, SALT_A, "a0"), row(1, SALT_A, "a1")]);
    const descending = chainAnchors([row(1, SALT_A, "a1"), row(0, SALT_A, "a0")]);
    expect(ascending).toEqual(descending);
  });

  it("counts a multi-tool turn ONCE (rows fan out, turns do not)", () => {
    const out = chainAnchors([row(0, SALT_A, "a0"), row(0, SALT_A, "a0"), row(1, SALT_A, "a1")]);
    expect(out[0]?.turns).toBe(2);
  });

  it("an UNSALTED chain is reconstructed, not discarded — it is the `kx agent run` shape", () => {
    // ⚠ Measured live: a real `kx agent run` produces rows with NO chain key at all.
    // The first version of this module skipped them as unscopable, which disabled the
    // whole surface for exactly the runs it exists to reach.
    const out = chainAnchors([row(0, "", "x0"), row(1, "", "x1"), row(2, "", "x2")]);
    expect(out).toHaveLength(1);
    expect(out[0]?.turn0MoteId).toBe("x0");
    expect(out[0]?.turnMoteIds).toEqual(["x0", "x1", "x2"]);
  });

  it("two UNSALTED chains are SEGMENTED by their turn restart, never merged", () => {
    // They share an instance id, carry no key and have no clock. Turn numbering is the
    // only signal that separates them — merging would invent one fictional run.
    const out = chainAnchors([
      row(0, "", "a0"),
      row(1, "", "a1"),
      row(0, "", "b0"),
      row(1, "", "b1"),
      row(2, "", "b2"),
    ]);
    expect(out).toHaveLength(2);
    expect(out.map((c) => c.turn0MoteId)).toEqual(["a0", "b0"]);
    expect(out[1]?.turns).toBe(3);
  });

  it("an empty page yields no anchors", () => {
    expect(chainAnchors([])).toEqual([]);
  });
});

describe("chainContaining — how the run view finds its OWN chain", () => {
  // A key lookup is not enough: `?chain=` is the chain key only when the server emitted
  // one, and for a plain `kx agent run` it is the turn-0 Mote instead. Membership finds
  // the right chain in BOTH shapes, which is the whole point.
  const chains = () =>
    chainAnchors([row(0, SALT_A, "a0"), row(1, SALT_A, "a1"), row(0, "", "u0"), row(1, "", "u1")]);

  it("finds a SALTED chain by its key", () => {
    expect(chainContaining(chains(), SALT_A)?.turn0MoteId).toBe("a0");
  });

  it("finds a chain by ANY of its turn Motes — including an unsalted one", () => {
    expect(chainContaining(chains(), "u1")?.turn0MoteId).toBe("u0");
    expect(chainContaining(chains(), "a1")?.turn0MoteId).toBe("a0");
  });

  it("returns null for a Mote in no chain, and for an empty anchor", () => {
    expect(chainContaining(chains(), "zz")).toBeNull();
    expect(chainContaining(chains(), "")).toBeNull();
  });

  it("never returns a DIFFERENT chain than the one the anchor belongs to", () => {
    const found = chainContaining(chains(), "u0");
    expect(found?.turnMoteIds).toEqual(["u0", "u1"]);
    expect(found?.turnMoteIds).not.toContain("a0");
  });
});

describe("mergeServerRuns — a CLI-launched run becomes scopable", () => {
  const server = [
    { instanceId: INSTANCE, recipeFingerprint: "ef".repeat(32), registeredUnixMs: 5 },
  ];

  it("expands a durable instance into ONE scopable row per agentic chain", () => {
    const out = mergeServerRuns(
      [],
      server,
      new Map([
        [
          INSTANCE,
          [
            { stepSalt: SALT_A, turn0MoteId: "a0" },
            { stepSalt: SALT_B, turn0MoteId: "b0" },
          ],
        ],
      ]),
    );
    expect(out).toHaveLength(2);
    expect(out.map((r) => [r.reactChainSalt, r.terminalMoteId])).toEqual([
      [SALT_A, "a0"],
      [SALT_B, "b0"],
    ]);
    // Distinct terminals ⇒ distinct row keys, so the table renders both.
    expect(new Set(out.map((r) => `${r.instanceId}:${r.terminalMoteId}`)).size).toBe(2);
  });

  it("keeps today's single UNSCOPED row when the instance has no chain", () => {
    // A plain pipeline, or a chain that aged out of the page. Not scopable, and the run
    // view must keep saying so rather than being handed a guess.
    const out = mergeServerRuns([], server, new Map());
    expect(out).toHaveLength(1);
    expect(out[0]?.terminalMoteId).toBeNull();
    expect(out[0]?.reactChainSalt).toBeNull();
  });

  it("without any chain map at all, behaviour is exactly as before", () => {
    const out = mergeServerRuns([], server);
    expect(out).toEqual(mergeServerRuns([], server, new Map()));
  });

  it("never displaces a local record — the richer per-invocation row wins", () => {
    const local = [
      {
        instanceId: INSTANCE,
        terminalMoteId: "local",
        reactChainSalt: null,
        recipeFingerprint: null,
        handle: "kx/recipes/react",
        startedAt: 9,
      },
    ];
    const out = mergeServerRuns(
      local,
      server,
      new Map([[INSTANCE, [{ stepSalt: SALT_A, turn0MoteId: "a0" }]]]),
    );
    expect(out).toHaveLength(1);
    expect(out[0]?.handle).toBe("kx/recipes/react");
  });
});
