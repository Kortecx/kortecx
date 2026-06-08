/** Parent-edge views (T3.3 DAG edges) — pure, no server. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { EdgeKind, ParentRefSchema } from "../src/gen/kortecx/v1/coordinator_pb.js";
import { MoteSnapshotSchema, MoteSnapshotState } from "../src/gen/kortecx/v1/gateway_pb.js";
import { ParentEdge, edgeKindName } from "../src/parents.js";
import { MoteView } from "../src/types.js";

const fill = (v: number, n: number): Uint8Array => new Uint8Array(n).fill(v);

describe("edgeKindName", () => {
  it("maps DATA/CONTROL and absorbs unknown", () => {
    expect(edgeKindName(EdgeKind.DATA)).toBe("data");
    expect(edgeKindName(EdgeKind.CONTROL)).toBe("control");
    expect(edgeKindName(EdgeKind.UNSPECIFIED)).toBe("unknown");
    expect(edgeKindName(99)).toBe("unknown");
  });
});

describe("ParentEdge.fromProto", () => {
  it("hex-encodes the parent id + maps edge meta", () => {
    const p = create(ParentRefSchema, {
      parentId: fill(0x06, 32),
      edgeKind: EdgeKind.CONTROL,
      nonCascade: true,
    });
    const e = ParentEdge.fromProto(p);
    expect(e.parentId).toBe("06".repeat(32));
    expect(e.edgeKind).toBe("control");
    expect(e.nonCascade).toBe(true);
  });
});

describe("MoteView parents", () => {
  it("populates parents from the snapshot", () => {
    const snap = create(MoteSnapshotSchema, {
      moteId: fill(0x03, 32),
      state: MoteSnapshotState.COMMITTED,
      moteDefHash: fill(0x05, 32),
      parents: [
        create(ParentRefSchema, {
          parentId: fill(0x01, 32),
          edgeKind: EdgeKind.DATA,
          nonCascade: false,
        }),
        create(ParentRefSchema, {
          parentId: fill(0x02, 32),
          edgeKind: EdgeKind.CONTROL,
          nonCascade: true,
        }),
      ],
    });
    const mv = MoteView.fromProto(snap);
    expect(mv.parents).toHaveLength(2);
    expect(mv.parents[0]).toEqual(new ParentEdge("01".repeat(32), "data", false));
    expect(mv.parents[1]?.edgeKind).toBe("control");
    expect(mv.parents[1]?.nonCascade).toBe(true);
  });

  it("defaults to an empty parents array (a root mote)", () => {
    const snap = create(MoteSnapshotSchema, {
      moteId: fill(0x03, 32),
      state: MoteSnapshotState.SCHEDULED,
      moteDefHash: fill(0x05, 32),
    });
    expect(MoteView.fromProto(snap).parents).toEqual([]);
    // a positional construction with no parents arg still works (additive/trailing).
    expect(new MoteView("aa", "S", 2, 1, 1, null, "bb", null, null).parents).toEqual([]);
  });

  it("toJSON() does NOT include parents (CLI byte-parity guard)", () => {
    const snap = create(MoteSnapshotSchema, {
      moteId: fill(0x03, 32),
      state: MoteSnapshotState.COMMITTED,
      moteDefHash: fill(0x05, 32),
      parents: [create(ParentRefSchema, { parentId: fill(0x01, 32), edgeKind: EdgeKind.DATA })],
    });
    const json = MoteView.fromProto(snap).toJSON();
    expect(Object.keys(json)).not.toContain("parents");
  });
});
