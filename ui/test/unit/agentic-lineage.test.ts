/**
 * The roster builder: `ListReactTurns` rows → the chain's node order.
 *
 * The ordering rule (dedupe by turn, first-wins over a newest-first wire) is
 * deliberately duplicated between this module and `use-react-progress`, because one is
 * in the eager entry chunk and the other is not. These tests are one half of what keeps
 * the two copies honest; `use-react-progress`'s own suite is the other.
 */

import { describe, expect, it } from "vitest";
import { agenticLineage } from "../../src/kx/use-projection";
import { REACT_LAUNCH, reactTurnRows, turnId } from "../mocks/projection-fixtures";

describe("agenticLineage", () => {
  it("orders a chain by turn, from a NEWEST-FIRST page", () => {
    const { turns } = reactTurnRows({ turns: 4 });
    expect(turns[0]?.turn).toBe(3); // the wire really is newest-first
    expect(agenticLineage(turns)).toEqual([turnId(0), turnId(1), turnId(2), turnId(3)]);
  });

  it("dedupes a multi-tool turn's rows to ONE roster entry", () => {
    // A turn that fires three tools at once fans into three rows sharing one turn Mote.
    const { turns } = reactTurnRows({ turns: 3, multiToolAt: 1, multiToolCount: 3 });
    expect(turns.filter((t) => t.turn === 1)).toHaveLength(3);
    expect(agenticLineage(turns)).toEqual([turnId(0), turnId(1), turnId(2)]);
  });

  it("first-wins over the wire: a settled row supersedes the same turn's pending one", () => {
    const { turns } = reactTurnRows({ turns: 2 });
    const pendingDuplicate = { ...turns[turns.length - 1], branch: "pending" } as (typeof turns)[0];
    // Append the stale pending row AFTER the settled one, as the wire's tail would.
    const roster = agenticLineage([...turns, pendingDuplicate]);
    expect(roster).toEqual([turnId(0), turnId(1)]);
  });

  it("with a resolved launch anchor, the ANSWER turn is replaced by the launch", () => {
    // The agentic-launch shape: the loop's answer commits onto the launch Mote, so
    // rendering the answer turn as well would show the same bytes on two nodes.
    const { turns } = reactTurnRows({ turns: 3, stepSalt: REACT_LAUNCH });
    const roster = agenticLineage(turns, REACT_LAUNCH);
    expect(roster).toEqual([turnId(0), turnId(1), REACT_LAUNCH]);
    expect(roster).not.toContain(turnId(2));
  });

  it("without a launch anchor, the ANSWER turn stays the terminal entry", () => {
    // `kx agent run`: the primary anchor is the discarded seed and never resolves, so
    // nothing absorbs the answer — and nothing else carries its bytes.
    const { turns } = reactTurnRows({ turns: 3 });
    expect(agenticLineage(turns)).toEqual([turnId(0), turnId(1), turnId(2)]);
  });

  it("a LIVE chain still terminates at the launch it will answer onto", () => {
    const { turns } = reactTurnRows({ turns: 3, stepSalt: REACT_LAUNCH, unsettled: true });
    const roster = agenticLineage(turns, REACT_LAUNCH);
    expect(roster.at(-1)).toBe(REACT_LAUNCH);
    expect(roster).toContain(turnId(2)); // not absorbed — it has not answered
  });

  it("an empty page yields an empty roster", () => {
    expect(agenticLineage([])).toEqual([]);
  });
});
