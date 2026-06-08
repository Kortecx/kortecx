/**
 * The parent-edge view — one inbound DAG edge of a Mote (hex parent id + edge
 * metadata). Kept in its own module so `types.ts` stays a thin aggregator,
 * mirroring the Rust core's module-per-concern discipline.
 *
 * SN-8: the parent id is server-derived; the SDK only *encodes* the bytes to
 * hex (never computes an id). An out-of-range `EdgeKind` renders `"unknown"` —
 * never a crash, never a silent mislabel (mirrors `stateName` in `types.ts`).
 */

import { EdgeKind } from "./gen/kortecx/v1/coordinator_pb.js";
import type { ParentRef } from "./gen/kortecx/v1/coordinator_pb.js";
import { encode } from "./hexids.js";

/** A parent edge's semantic kind. `"unknown"` absorbs UNSPECIFIED(0) + any future value. */
export type EdgeKindName = "data" | "control" | "unknown";

/** Map an `EdgeKind` discriminant to a stable name (`"unknown"` if new). */
export function edgeKindName(kind: number): EdgeKindName {
  if (kind === EdgeKind.DATA) return "data";
  if (kind === EdgeKind.CONTROL) return "control";
  return "unknown";
}

/** One inbound edge of a Mote in the projection DAG (hex parent id + edge meta). */
export class ParentEdge {
  constructor(
    readonly parentId: string,
    readonly edgeKind: EdgeKindName,
    readonly nonCascade: boolean,
  ) {}

  static fromProto(p: ParentRef): ParentEdge {
    return new ParentEdge(encode(p.parentId), edgeKindName(p.edgeKind), p.nonCascade);
  }
}
