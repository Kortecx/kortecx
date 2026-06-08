import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { allTerminal, toProjectionVM, useProjection } from "../../src/kx/use-projection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";
import { mote, projection } from "../mocks/projection-fixtures";

const INSTANCE = "ab".repeat(16);

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

describe("useProjection", () => {
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
