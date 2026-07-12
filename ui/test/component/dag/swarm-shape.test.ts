/**
 * PR-B: pure swarm-shape inference over the projection — gather detection, branch
 * membership, the RPC-free majority winner (resultRef equality) + agreement count,
 * and the honest `consensus`/`parallel` label. No rendering.
 */

import { describe, expect, it } from "vitest";
import { branchEdgeIds, detectSwarm } from "../../../src/components/dag/swarm-shape";
import { toProjectionVM } from "../../../src/kx/use-projection";
import {
  type MoteOpts,
  chainProjection,
  diamondProjection,
  fanInProjection,
  mote,
  nid,
  projection,
} from "../../mocks/projection-fixtures";

/** Build the plain MoteVM[] the detector consumes from fixture Motes. */
function motesVM(opts: MoteOpts[]) {
  return toProjectionVM(projection(opts.map((o) => mote(o)))).motes;
}
const vmOf = (p: ReturnType<typeof fanInProjection>) => toProjectionVM(p).motes;

describe("detectSwarm (PR-B)", () => {
  it("returns null for a linear run (no ≥2-way data fan-in)", () => {
    expect(detectSwarm(toProjectionVM(chainProjection(4)).motes)).toBeNull();
  });

  it("detects the gather + its branches for a fan-in (parallel, no agreement)", () => {
    const shape = detectSwarm(vmOf(fanInProjection(3)));
    expect(shape).not.toBeNull();
    expect(shape?.gatherId).toBe(nid(3));
    expect(shape?.branches.map((b) => b.moteId)).toEqual([nid(0), nid(1), nid(2)]);
    expect(shape?.pattern).toBe("parallel");
    expect(shape?.agreementCount).toBe(0);
  });

  it("emits the branch→gather edge ids for highlighting", () => {
    const shape = detectSwarm(vmOf(fanInProjection(3)));
    expect([...branchEdgeIds(shape)].sort()).toEqual(
      [`${nid(0)}->${nid(3)}`, `${nid(1)}->${nid(3)}`, `${nid(2)}->${nid(3)}`].sort(),
    );
    expect([...branchEdgeIds(null)]).toEqual([]);
  });

  it("a diamond is a minimal 2-branch fan-in", () => {
    const shape = detectSwarm(toProjectionVM(diamondProjection()).motes);
    expect(shape?.gatherId).toBe(nid(3));
    expect(shape?.branches).toHaveLength(2);
  });

  it("marks the majority winner(s) + labels consensus when ≥2 branches agree", () => {
    const WIN = nid(99);
    const shape = detectSwarm(
      motesVM([
        { moteId: nid(1), resultRef: WIN, parents: [] },
        { moteId: nid(2), resultRef: WIN, parents: [] },
        { moteId: nid(3), resultRef: nid(88), parents: [] },
        {
          moteId: nid(4),
          resultRef: WIN,
          parents: [{ parentId: nid(1) }, { parentId: nid(2) }, { parentId: nid(3) }],
        },
      ]),
    );
    expect(shape?.pattern).toBe("consensus");
    expect(shape?.agreementCount).toBe(2);
    expect(
      shape?.branches
        .filter((b) => b.won)
        .map((b) => b.moteId)
        .sort(),
    ).toEqual([nid(1), nid(2)].sort());
  });

  it("no winner while the gather is uncommitted (resultRef null → parallel)", () => {
    const shape = detectSwarm(
      motesVM([
        { moteId: nid(1), resultRef: nid(7), parents: [] },
        { moteId: nid(2), resultRef: nid(7), parents: [] },
        { moteId: nid(3), resultRef: null, parents: [{ parentId: nid(1) }, { parentId: nid(2) }] },
      ]),
    );
    expect(shape?.agreementCount).toBe(0);
    expect(shape?.pattern).toBe("parallel");
  });

  it("picks the WIDEST fan-in as the primary gather", () => {
    const shape = detectSwarm(
      motesVM([
        { moteId: nid(1), parents: [] },
        { moteId: nid(2), parents: [] },
        { moteId: nid(3), parents: [] },
        // gather A: 2 branches
        { moteId: nid(10), parents: [{ parentId: nid(1) }, { parentId: nid(2) }] },
        // gather B: 3 branches → primary
        {
          moteId: nid(11),
          parents: [{ parentId: nid(1) }, { parentId: nid(2) }, { parentId: nid(3) }],
        },
      ]),
    );
    expect(shape?.gatherId).toBe(nid(11));
    expect(shape?.branches).toHaveLength(3);
  });

  it("ignores dangling (absent) and control-edge parents", () => {
    // Only 1 present DATA parent (nid(2)); nid(3) is control, nid(99) is absent → not a gather.
    const shape = detectSwarm(
      motesVM([
        { moteId: nid(2), parents: [] },
        { moteId: nid(3), parents: [] },
        {
          moteId: nid(5),
          parents: [
            { parentId: nid(2), edgeKind: "data" },
            { parentId: nid(3), edgeKind: "control" },
            { parentId: nid(99), edgeKind: "data" },
          ],
        },
      ]),
    );
    expect(shape).toBeNull();
  });
});
