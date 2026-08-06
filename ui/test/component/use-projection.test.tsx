import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  allTerminal,
  isRunAtRest,
  runSettled,
  scopeProjection,
  toProjectionVM,
  useProjection,
} from "../../src/kx/use-projection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";
import {
  ANSWER_REF,
  REACT_LAUNCH,
  REACT_SEED,
  mote,
  obsId,
  projection,
  reactChainProjection,
  reactTurnRows,
  turnId,
} from "../mocks/projection-fixtures";

const INSTANCE = "ab".repeat(16);
const TERMINAL = "ee".repeat(32);

describe("toProjectionVM", () => {
  it("maps every field the views need", () => {
    const vm = toProjectionVM(
      projection(
        [mote({ moteId: "11".repeat(32), stateCode: 4, ndClass: 3, committedSeq: 7, anomaly: 1 })],
        {
          currentSeq: 9,
        },
      ),
    );
    expect(vm.currentSeq).toBe(9);
    expect(vm.motes).toHaveLength(1);
    expect(vm.motes[0]).toMatchObject({
      moteId: "11".repeat(32),
      stateCode: 4,
      ndClass: 3,
      committedSeq: 7,
      anomaly: 1,
    });
  });

  it("maps parent edges (the DAG links) and defaults to [] for a root", () => {
    const vm = toProjectionVM(
      projection([
        mote({ moteId: "00".repeat(32) }),
        mote({
          moteId: "01".repeat(32),
          parents: [
            { parentId: "00".repeat(32), edgeKind: "data" },
            { parentId: "00".repeat(32), edgeKind: "control", nonCascade: true },
          ],
        }),
      ]),
    );
    expect(vm.motes[0]?.parents).toEqual([]); // a root
    expect(vm.motes[1]?.parents).toEqual([
      { parentId: "00".repeat(32), edgeKind: "data", nonCascade: false },
      { parentId: "00".repeat(32), edgeKind: "control", nonCascade: true },
    ]);
  });
});

describe("allTerminal", () => {
  it("false for an empty projection", () => {
    expect(allTerminal(toProjectionVM(projection([])))).toBe(false);
  });
  it("false while any Mote is in-flight", () => {
    const vm = toProjectionVM(projection([mote({ stateCode: 3 }), mote({ stateCode: 2 })]));
    expect(allTerminal(vm)).toBe(false);
  });
  it("true once every Mote is terminal", () => {
    const vm = toProjectionVM(projection([mote({ stateCode: 3 }), mote({ stateCode: 4 })]));
    expect(allTerminal(vm)).toBe(true);
  });
});

describe("isRunAtRest (the poll-stop signal)", () => {
  it("with a terminal id: stays live until the terminal Mote commits (even while children register)", () => {
    // Only the root is visible + committed and the frontier is stable — but the
    // terminal (sink) Mote has not appeared yet → keep polling (the bug a naive
    // all-terminal check hits: it would stop here at one node).
    const early = toProjectionVM(
      projection([mote({ moteId: "aa".repeat(32), stateCode: 3 })], { currentSeq: 3 }),
    );
    expect(isRunAtRest(early, TERMINAL, 3)).toBe(false);

    // Terminal Mote present + COMMITTED → at rest.
    const done = toProjectionVM(
      projection(
        [mote({ moteId: "aa".repeat(32), stateCode: 3 }), mote({ moteId: TERMINAL, stateCode: 3 })],
        { currentSeq: 11 },
      ),
    );
    expect(isRunAtRest(done, TERMINAL, 11)).toBe(true);

    // Terminal Mote present but still SCHEDULED → keep polling.
    const running = toProjectionVM(projection([mote({ moteId: TERMINAL, stateCode: 2 })]));
    expect(isRunAtRest(running, TERMINAL, 9)).toBe(false);
  });

  it("without a terminal id: frontier-stability fallback (all terminal + seq unchanged)", () => {
    const vm = toProjectionVM(projection([mote({ stateCode: 3 })], { currentSeq: 7 }));
    expect(isRunAtRest(vm, undefined, 7)).toBe(true); // settled
    expect(isRunAtRest(vm, undefined, 6)).toBe(false); // frontier advanced this poll → keep polling
    const inFlight = toProjectionVM(projection([mote({ stateCode: 2 })], { currentSeq: 7 }));
    expect(isRunAtRest(inFlight, undefined, 7)).toBe(false); // a Mote still in flight
  });

  it("runSettled prefers the terminal-Mote signal over all-terminal", () => {
    const vm = toProjectionVM(
      projection([
        mote({ moteId: "aa".repeat(32), stateCode: 3 }),
        mote({ moteId: TERMINAL, stateCode: 2 }), // the gather is still scheduled
      ]),
    );
    expect(allTerminal(vm)).toBe(false);
    expect(runSettled(vm, TERMINAL)).toBe(false);
  });
});

/**
 * The scope is the difference between "this run" and "everything this gateway ever ran":
 * one `kx serve` is ONE journal with ONE instance id shared by every submission. These
 * pin the three outcomes a caller has to tell apart — no anchor, a good anchor, and an
 * anchor that isn't there — because #362 shipped the machinery with none of them tested
 * and the wiring turned out to be dead.
 */
describe("scopeProjection", () => {
  const A1 = "a1".repeat(32);
  const A2 = "a2".repeat(32);
  const B1 = "b1".repeat(32);
  /** Two unrelated runs folded from one journal — what a shared serve accumulates. */
  const journal = () =>
    toProjectionVM(
      projection(
        [
          mote({ moteId: A1 }),
          mote({ moteId: A2, parents: [{ parentId: A1 }] }),
          mote({ moteId: B1 }),
        ],
        { currentSeq: 3 },
      ),
    );

  it("no anchor ⇒ the whole fold, and scopeMissed is FALSE", () => {
    // "Unscoped" is not a failure — it is a caller that never asked. The view still has
    // to say what it is showing, but `scopeMissed` must not claim a lookup failed.
    const out = scopeProjection(journal());
    expect(out.motes.map((m) => m.moteId)).toEqual([A1, A2, B1]);
    expect(out.scopeMissed).toBe(false);
  });

  it("an anchor in the fold ⇒ only that run's connected component", () => {
    const out = scopeProjection(journal(), A2);
    expect(out.motes.map((m) => m.moteId)).toEqual([A1, A2]);
    expect(out.scopeMissed).toBe(false);
    // The instance-level facts are the journal's and are carried through untouched.
    expect(out.currentSeq).toBe(3);
  });

  it("an anchor that is NOT in the fold ⇒ scopeMissed, with the motes left UNSCOPED", () => {
    // A stale link, or a journal rebuilt under the same endpoint.
    //
    // ⚠ PINNING A SHARP EDGE, not endorsing it: the narrowing is DROPPED here, not
    // applied-and-emptied, so `motes` still holds every run in the journal. The flag is
    // therefore the ONLY thing between the user and somebody else's run being presented
    // as theirs — every consumer must branch on `scopeMissed` BEFORE it reads `motes`
    // (the run view's notice, ArtifactGallery, the export + clone refusals all do).
    const out = scopeProjection(journal(), "ff".repeat(32));
    expect(out.scopeMissed).toBe(true);
    expect(out.motes.map((m) => m.moteId)).toEqual([A1, A2, B1]);
  });

  it("an EMPTY anchor is 'not asked', not 'not found'", () => {
    // `runAnchor()` returns "" when the server gave us neither key, and several call
    // sites forward it straight through. That has to read as unscoped.
    const out = scopeProjection(journal(), "");
    expect(out.scopeMissed).toBe(false);
    expect(out.motes).toHaveLength(3);
  });

  it("a missed anchor with a resolvable FALLBACK ⇒ the fallback's component, no banner", () => {
    // A react Invoke's `chain=` carries the chain SALT (the Timeline's
    // ListReactTurns key). From a server that returned the discarded seed as the
    // anchor, the salt resolves to nothing — the run view then retries with
    // `terminal` (the admitted turn-0) so the user gets their run rather than the
    // whole-journal notice, and the Timeline keeps the salt it needs.
    const out = scopeProjection(journal(), "ff".repeat(32), B1);
    expect(out.scopeMissed).toBe(false);
    expect(out.motes.map((m) => m.moteId)).toEqual([B1]);
  });

  it("BOTH anchors missing ⇒ scopeMissed, honestly", () => {
    const out = scopeProjection(journal(), "ff".repeat(32), "ee".repeat(32));
    expect(out.scopeMissed).toBe(true);
    expect(out.motes).toHaveLength(3);
  });

  it("a resolvable PRIMARY ignores the fallback", () => {
    const out = scopeProjection(journal(), A2, B1);
    expect(out.motes.map((m) => m.moteId)).toEqual([A1, A2]);
    expect(out.scopeMissed).toBe(false);
  });
});

describe("useProjection", () => {
  it("scopes the query to one run; an UNSCOPED call is a different cache entry", async () => {
    const A1 = "a1".repeat(32);
    const A2 = "a2".repeat(32);
    const B1 = "b1".repeat(32);
    const { client } = makeMockClient({
      getProjection: async () =>
        projection([
          mote({ moteId: A1, stateCode: 3 }),
          mote({ moteId: A2, stateCode: 3, parents: [{ parentId: A1 }] }),
          mote({ moteId: B1, stateCode: 3 }),
        ]),
    });
    const wrapper = connectedWrapper(client);
    const scoped = renderHook(() => useProjection(INSTANCE, { scopeMoteId: A2 }), { wrapper });
    await waitFor(() => expect(scoped.result.current.data).toBeTruthy());
    expect(scoped.result.current.data?.motes.map((m) => m.moteId)).toEqual([A1, A2]);
    expect(scoped.result.current.data?.scopeMissed).toBe(false);

    // Same instance, no scope: `scopeMoteId` is part of the query key, so this is a
    // separate entry holding the whole journal — which is exactly how the graph could
    // show 4 steps while the Artifacts tab beside it listed the workspace.
    const unscoped = renderHook(() => useProjection(INSTANCE), { wrapper });
    await waitFor(() => expect(unscoped.result.current.data).toBeTruthy());
    expect(unscoped.result.current.data?.motes).toHaveLength(3);
  });

  it("reports scopeMissed when the anchor is absent from the fold", async () => {
    const { client } = makeMockClient({
      getProjection: async () => projection([mote({ moteId: "aa".repeat(32), stateCode: 3 })]),
    });
    const { result } = renderHook(() => useProjection(INSTANCE, { scopeMoteId: "ff".repeat(32) }), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(result.current.data?.scopeMissed).toBe(true);
  });

  it("loads a projection from the gateway", async () => {
    const { client, getProjection } = makeMockClient({
      getProjection: async () => projection([mote({ stateCode: 3 })], { currentSeq: 5 }),
    });
    const { result } = renderHook(() => useProjection(INSTANCE), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(result.current.data?.currentSeq).toBe(5);
    expect(getProjection).toHaveBeenCalled();
  });

  it("keeps a stable data reference across an unchanged poll (no re-render churn)", async () => {
    const { client } = makeMockClient({
      // New Projection instance each call, but identical content.
      getProjection: async () =>
        projection([mote({ moteId: "aa".repeat(32), stateCode: 2 })], { currentSeq: 5 }),
    });
    const { result } = renderHook(() => useProjection(INSTANCE), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.data).toBeTruthy());
    const first = result.current.data;
    await act(async () => {
      await result.current.refetch();
    });
    // Structural sharing returns the prior reference when content is unchanged.
    expect(result.current.data).toBe(first);
  });

  it("reflects an advancing frontier (a Mote flips SCHEDULED → COMMITTED)", async () => {
    const frames = [
      projection([mote({ moteId: "bb".repeat(32), stateCode: 2 })], { currentSeq: 5 }),
      projection([mote({ moteId: "bb".repeat(32), stateCode: 3 })], { currentSeq: 6 }),
    ];
    let i = 0;
    const { client, getProjection } = makeMockClient({
      getProjection: async () => frames[Math.min(i++, frames.length - 1)],
    });
    const { result } = renderHook(() => useProjection(INSTANCE), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.data?.currentSeq).toBe(5));
    const first = result.current.data;
    await act(async () => {
      await result.current.refetch();
    });
    await waitFor(() => expect(result.current.data?.currentSeq).toBe(6));
    expect(getProjection.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(result.current.data?.motes[0]?.stateCode).toBe(3);
    // Content changed → structural sharing yields a fresh reference.
    expect(result.current.data).not.toBe(first);
  });
});

describe("useProjection — agentic lineage", () => {
  // A react/agentic run folds as DISJOINT TWO-NODE STARS: the coordinator registers each
  // turn Mote EDGE-FREE (declaring parents would move the canonical digest) and only the
  // tool observation carries a Data edge, back to its own turn. So an undirected walk from
  // any anchor reaches exactly one star, and the graph has never shown more than that.
  // The lineage IS durable — it is just off-DAG, in the ReactRound facts ListReactTurns
  // serves. These tests pin that the reader joins the two.

  it("kx agent run: scopes the WHOLE chain, not just turn 0", async () => {
    // `?chain=` is the react chain salt = the seed the coordinator validates then DISCARDS,
    // so the primary anchor ALWAYS misses and the `terminal` fallback (the admitted turn-0)
    // is what resolves. Today that yields turn 0's star and nothing else.
    const { client, listReactTurns } = makeMockClient({
      getProjection: async () => reactChainProjection({ turns: 3 }),
      listReactTurns: async () => reactTurnRows({ turns: 3, stepSalt: REACT_SEED }),
    });
    const { result } = renderHook(
      () => useProjection(INSTANCE, { scopeMoteId: REACT_SEED, scopeFallbackMoteId: turnId(0) }),
      { wrapper: connectedWrapper(client) },
    );
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(result.current.data?.motes.map((m) => m.moteId)).toEqual([
      turnId(0),
      obsId(0),
      turnId(1),
      obsId(1),
      turnId(2),
    ]);
    expect(result.current.data?.scopeMissed).toBe(false);
    // Requested WITHOUT a chain key and selected by membership — see the
    // "the shape a real `kx agent run` actually produces" block below for why.
    expect(listReactTurns).toHaveBeenCalledWith({ instanceId: INSTANCE, limit: 500 });
  });

  it("kx chat --tools: the answer turn is ABSORBED by the launch, so its bytes render once", async () => {
    // Here the salt IS an admitted Mote (the agentic launch step), and the loop's answer
    // commits ONTO it. Rendering the answer turn as well would show the same bytes twice.
    const { client } = makeMockClient({
      getProjection: async () => reactChainProjection({ turns: 3, launch: true }),
      listReactTurns: async () => reactTurnRows({ turns: 3, stepSalt: REACT_LAUNCH }),
    });
    const { result } = renderHook(() => useProjection(INSTANCE, { scopeMoteId: REACT_LAUNCH }), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.data).toBeTruthy());
    const ids = result.current.data?.motes.map((m) => m.moteId) ?? [];
    expect(ids).toContain(REACT_LAUNCH);
    expect(ids).toContain(turnId(0));
    expect(ids).toContain(obsId(0));
    // The answer turn's Mote is NOT a node — the launch already carries its bytes.
    expect(ids).not.toContain(turnId(2));
    expect(result.current.data?.motes.filter((m) => m.resultRef === ANSWER_REF)).toHaveLength(1);
    // The roster still ends at the launch, so the chain has somewhere to terminate.
    expect(result.current.data?.agenticTurnIds?.at(-1)).toBe(REACT_LAUNCH);
  });

  it("a turn whose Mote has not reached the frontier is DROPPED, never faked", async () => {
    // Mid-poll: three turn rows exist but only two turn Motes have landed. A placeholder
    // would have to invent a state code, and stateCode 0 renders UNKNOWN — a lie about a
    // Mote that is merely not in our copy of the fold yet.
    const { client } = makeMockClient({
      getProjection: async () => reactChainProjection({ turns: 3, presentTurns: 2 }),
      listReactTurns: async () => reactTurnRows({ turns: 3, stepSalt: REACT_SEED }),
    });
    const { result } = renderHook(
      () => useProjection(INSTANCE, { scopeMoteId: REACT_SEED, scopeFallbackMoteId: turnId(0) }),
      { wrapper: connectedWrapper(client) },
    );
    await waitFor(() => expect(result.current.data).toBeTruthy());
    const ids = result.current.data?.motes.map((m) => m.moteId) ?? [];
    expect(ids).toEqual([turnId(0), obsId(0), turnId(1), obsId(1)]);
    expect(ids).not.toContain(turnId(2));
  });

  it("does NOT call ListReactTurns when there is no scope anchor", async () => {
    const { client, listReactTurns } = makeMockClient({
      getProjection: async () => projection([mote({ stateCode: 3 })]),
    });
    const { result } = renderHook(() => useProjection(INSTANCE), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(listReactTurns).not.toHaveBeenCalled();
  });

  it("degrades to today's scope when ListReactTurns is unavailable", async () => {
    // An older gateway answers UNIMPLEMENTED. That must narrow exactly as before — never
    // fail the projection, which would blank a run view that used to render.
    const { client } = makeMockClient({
      getProjection: async () => reactChainProjection({ turns: 3 }),
      listReactTurns: async () => {
        throw new Error("UNIMPLEMENTED: ListReactTurns");
      },
    });
    const { result } = renderHook(
      () => useProjection(INSTANCE, { scopeMoteId: REACT_SEED, scopeFallbackMoteId: turnId(0) }),
      { wrapper: connectedWrapper(client) },
    );
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(result.current.error).toBeFalsy();
    expect(result.current.data?.motes.map((m) => m.moteId)).toEqual([turnId(0), obsId(0)]);
    expect(result.current.data?.scopeMissed).toBe(false);
  });

  it("rows whose Motes are ALL absent do not clear scopeMissed", async () => {
    // The honest-degrade path must survive the widening: if neither URL anchor nor any
    // turn Mote is in the fold, the view must still say it could not isolate the run.
    const { client } = makeMockClient({
      getProjection: async () => projection([mote({ moteId: "cc".repeat(32), stateCode: 3 })]),
      listReactTurns: async () => reactTurnRows({ turns: 3, stepSalt: REACT_SEED }),
    });
    const { result } = renderHook(
      () => useProjection(INSTANCE, { scopeMoteId: REACT_SEED, scopeFallbackMoteId: turnId(0) }),
      { wrapper: connectedWrapper(client) },
    );
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(result.current.data?.scopeMissed).toBe(true);
  });

  it("a GROWING roster stays one cache entry (no refetch storm)", async () => {
    // The roster is derived INSIDE the query fn precisely so it cannot enter the query
    // key. If it did, every arriving turn would mint a new cache entry and flash loading.
    const frames = [reactTurnRows({ turns: 2 }), reactTurnRows({ turns: 4 })];
    let i = 0;
    const { client } = makeMockClient({
      getProjection: async () => reactChainProjection({ turns: 4 }),
      listReactTurns: async () => frames[Math.min(i++, frames.length - 1)],
    });
    const { result } = renderHook(
      () => useProjection(INSTANCE, { scopeMoteId: REACT_SEED, scopeFallbackMoteId: turnId(0) }),
      { wrapper: connectedWrapper(client) },
    );
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(result.current.data?.motes.map((m) => m.moteId)).toEqual([
      turnId(0),
      obsId(0),
      turnId(1),
      obsId(1),
    ]);
    await act(async () => {
      await result.current.refetch();
    });
    await waitFor(() => expect(result.current.data?.motes.length).toBe(7));
    expect(result.current.isLoading).toBe(false);
  });
});

describe("useProjection — the shape a real `kx agent run` actually produces", () => {
  /**
   * ⚠ THE REGRESSION GUARD FOR A LIVE FINDING. The first implementation asked for the
   * chain BY KEY, using the run view's `?chain=` as the key. That reads as obviously
   * right and is wrong for the case this whole surface exists for: measured against a
   * real `kx agent run` on a served model, every row of a 3-turn chain carried NO chain
   * key at all, and `?chain=` was the turn-0 Mote. A keyed request matched nothing, and
   * the graph stayed at the 2 nodes it had always shown — with `ListReactTurns` visibly
   * being called, which is what made it look fixed.
   *
   * The unit suite could not see this, because the mock returned rows regardless of the
   * request. These tests drive the mock from the REQUEST, so a lookup that cannot match
   * comes back empty exactly as the server would answer it.
   */
  function unsaltedClient() {
    const page = reactTurnRows({ turns: 3, stepSalt: "" });
    return makeMockClient({
      getProjection: async () => reactChainProjection({ turns: 3 }),
      // Honour the filter, as the server does.
      listReactTurns: async (req: unknown) => {
        const salt = (req as { stepSalt?: string } | undefined)?.stepSalt;
        if (salt) {
          return { turns: [], hasMore: false }; // no row carries a key
        }
        return page;
      },
    });
  }

  it("scopes the whole chain when the anchor is the TURN-0 Mote and rows carry no key", async () => {
    const { client } = unsaltedClient();
    const { result } = renderHook(
      // What `runViewSearch` emits for a run with no salt: chain === terminal === turn 0.
      () => useProjection(INSTANCE, { scopeMoteId: turnId(0), scopeFallbackMoteId: turnId(0) }),
      { wrapper: connectedWrapper(client) },
    );
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(result.current.data?.motes.map((m) => m.moteId)).toEqual([
      turnId(0),
      obsId(0),
      turnId(1),
      obsId(1),
      turnId(2),
    ]);
    expect(result.current.data?.agenticTurnIds).toEqual([turnId(0), turnId(1), turnId(2)]);
  });

  it("asks for the chain WITHOUT a key — a keyed request cannot match this shape", async () => {
    const { client, listReactTurns } = unsaltedClient();
    renderHook(() => useProjection(INSTANCE, { scopeMoteId: turnId(0) }), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(listReactTurns).toHaveBeenCalled());
    for (const call of listReactTurns.mock.calls) {
      expect((call[0] as { stepSalt?: string })?.stepSalt).toBeUndefined();
    }
  });

  it("selects by MEMBERSHIP, so another run's chain is never adopted", async () => {
    // Two unsalted chains under one instance. The anchor belongs to the second; the
    // first must not contribute a single node.
    const mine = reactTurnRows({ turns: 3, stepSalt: "" });
    const foreign = {
      turns: mine.turns.map(
        (t) => ({ ...t, turnMoteId: `ff${t.turnMoteId.slice(2)}`, seq: t.seq - 100 }) as typeof t,
      ),
      hasMore: false as const,
    };
    const { client } = makeMockClient({
      getProjection: async () => reactChainProjection({ turns: 3 }),
      listReactTurns: async () => ({
        turns: [...mine.turns, ...foreign.turns],
        hasMore: false,
      }),
    });
    const { result } = renderHook(
      () => useProjection(INSTANCE, { scopeMoteId: turnId(0), scopeFallbackMoteId: turnId(0) }),
      { wrapper: connectedWrapper(client) },
    );
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(result.current.data?.agenticTurnIds).toEqual([turnId(0), turnId(1), turnId(2)]);
    expect(result.current.data?.motes.every((m) => !m.moteId.startsWith("ff"))).toBe(true);
  });
});
