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
import { mote, projection } from "../mocks/projection-fixtures";

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
