/**
 * D155 branch views — a named, content-addressed `{path -> ContentRef}` manifest
 * over operator-approved host files. A caller snapshots confined host files
 * (under `KX_SERVE_FS_ROOT`, default-OFF) INTO the content store and the agent
 * loop edits them IN-CAS (the host is never written in Phase-A). Kept in its own
 * module so `types.ts` stays a thin aggregator (GR3).
 *
 * SN-8: `branchRef` is SERVER-DERIVED (blake3 over the manifest) — the client
 * names a handle, never an identity. The manifest lives in an off-journal
 * `branches.db` sidecar (rebuildable-to-empty), scoped to the authoring party; a
 * not-found / not-owned branch is UNIFORM (no cross-party existence oracle).
 */

import type {
  Branch as PbBranch,
  BranchItem as PbBranchItem,
  CreateBranchResponse as PbCreateBranchResponse,
  SnapshotIntoResponse as PbSnapshotIntoResponse,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One manifest entry: a snapshot-relative path + its content-store ref. */
export class BranchItem {
  constructor(
    readonly path: string,
    /** The 32-byte content-store ref, as 64 hex chars. */
    readonly contentRef: string,
  ) {}

  static fromProto(it: PbBranchItem): BranchItem {
    return new BranchItem(it.path, encode(it.contentRef));
  }

  toJSON() {
    return { path: this.path, content_ref: this.contentRef };
  }
}

/** A branch's resolved manifest (the governance / display view + edit source). */
export class Branch {
  constructor(
    /** The server-derived manifest hash, as hex. */
    readonly branchRef: string,
    readonly handle: string,
    /** The CoW parent handle (lineage); "" = a root branch. */
    readonly parentHandle: string,
    readonly description: string,
    readonly items: BranchItem[],
    readonly itemCount: number,
  ) {}

  static fromProto(b: PbBranch): Branch {
    return new Branch(
      encode(b.branchRef),
      b.handle,
      b.parentHandle,
      b.description,
      b.items.map((it) => BranchItem.fromProto(it)),
      b.itemCount,
    );
  }

  toJSON() {
    return {
      branch_ref: this.branchRef,
      handle: this.handle,
      parent_handle: this.parentHandle,
      description: this.description,
      item_count: this.itemCount,
      items: this.items.map((i) => i.toJSON()),
    };
  }
}

/** The outcome of a `CreateBranch` upsert (server-derived ref + dedup flag). */
export class CreateBranchResult {
  constructor(
    readonly branchRef: string,
    readonly handle: string,
    readonly deduplicated: boolean,
  ) {}

  static fromProto(r: PbCreateBranchResponse): CreateBranchResult {
    return new CreateBranchResult(encode(r.branchRef), r.handle, r.deduplicated);
  }

  toJSON() {
    return { branch_ref: this.branchRef, handle: this.handle, deduplicated: this.deduplicated };
  }
}

/** The outcome of a `SnapshotInto` — the resolved manifest + the ingest count. */
export class SnapshotResult {
  constructor(
    readonly branchRef: string,
    readonly handle: string,
    readonly ingested: number,
    readonly items: BranchItem[],
    readonly deduplicated: boolean,
  ) {}

  static fromProto(r: PbSnapshotIntoResponse): SnapshotResult {
    return new SnapshotResult(
      encode(r.branchRef),
      r.handle,
      r.ingested,
      r.items.map((it) => BranchItem.fromProto(it)),
      r.deduplicated,
    );
  }

  toJSON() {
    return {
      branch_ref: this.branchRef,
      handle: this.handle,
      ingested: this.ingested,
      deduplicated: this.deduplicated,
      items: this.items.map((i) => i.toJSON()),
    };
  }
}
