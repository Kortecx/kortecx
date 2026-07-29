/**
 * D155 branch views — a named, content-addressed `{path -> ContentRef}` manifest
 * over operator-approved host files. A caller snapshots confined host files
 * (under `KX_SERVE_FS_ROOT`, default-OFF) INTO the content store and the agent
 * loop edits them IN-CAS (the host is never written in Phase-A). Kept in its own
 * module so `types.ts` stays a thin aggregator.
 *
 * `branchRef` is SERVER-DERIVED (blake3 over the manifest) — the client
 * names a handle, never an identity. The manifest lives in an off-journal
 * `branches.db` sidecar (rebuildable-to-empty), scoped to the authoring party; a
 * not-found / not-owned branch is UNIFORM (no cross-party existence oracle).
 */

import type {
  AdvanceBranchResponse as PbAdvanceBranchResponse,
  Branch as PbBranch,
  BranchItem as PbBranchItem,
  BranchVersion as PbBranchVersion,
  CreateBranchResponse as PbCreateBranchResponse,
  RestoreBranchResponse as PbRestoreBranchResponse,
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

/** The outcome of an `AdvanceBranch` (D155 Phase-3) — the manifest after the
 * in-CAS re-point. `deduplicated` is true iff the re-point was a no-op. */
export class AdvanceResult {
  constructor(
    readonly branchRef: string,
    readonly handle: string,
    readonly items: BranchItem[],
    readonly deduplicated: boolean,
  ) {}

  static fromProto(r: PbAdvanceBranchResponse): AdvanceResult {
    return new AdvanceResult(
      encode(r.branchRef),
      r.handle,
      r.items.map((it) => BranchItem.fromProto(it)),
      r.deduplicated,
    );
  }

  toJSON() {
    return {
      branch_ref: this.branchRef,
      handle: this.handle,
      deduplicated: this.deduplicated,
      items: this.items.map((i) => i.toJSON()),
    };
  }
}

/** One recorded point-in-time of a branch manifest. Every non-dedup mutation
 * (create / snapshot / advance / restore) appends a version; the CAS blobs
 * behind a recorded version are never collected by branch ops, so any listed
 * version is restorable. */
export class BranchVersion {
  constructor(
    /** 1-based per-handle version, monotone (1 = oldest retained). */
    readonly version: number,
    /** The server-derived manifest hash at this version, as hex (display only). */
    readonly branchRef: string,
    /** Sidecar wall-clock at record time (ms since epoch); advisory. */
    readonly recordedUnixMs: number,
    /** "baseline" | "create" | "snapshot" | "advance" | "restore". */
    readonly cause: string,
    /** Manifest size at this version. */
    readonly itemCount: number,
  ) {}

  static fromProto(v: PbBranchVersion): BranchVersion {
    return new BranchVersion(
      v.version,
      encode(v.branchRef),
      Number(v.recordedUnixMs),
      v.cause,
      v.itemCount,
    );
  }

  toJSON() {
    return {
      version: this.version,
      branch_ref: this.branchRef,
      recorded_unix_ms: this.recordedUnixMs,
      cause: this.cause,
      item_count: this.itemCount,
    };
  }
}

/** The outcome of a `RestoreBranch` — restore APPENDS a new version whose items
 * are the historical items (history is never rewound; a restore is itself
 * history). `deduplicated` is true iff the branch already matched the requested
 * version (no-op, nothing recorded, `newVersion` = 0). */
export class RestoreResult {
  constructor(
    readonly branch: Branch,
    readonly newVersion: number,
    readonly deduplicated: boolean,
  ) {}

  static fromProto(r: PbRestoreBranchResponse): RestoreResult {
    if (r.branch === undefined) {
      throw new Error("RestoreBranch: response carried no branch");
    }
    return new RestoreResult(Branch.fromProto(r.branch), r.newVersion, r.deduplicated);
  }

  toJSON() {
    return {
      branch: this.branch.toJSON(),
      new_version: this.newVersion,
      deduplicated: this.deduplicated,
    };
  }
}
