/**
 * The sharing (grants) views — every grant on an asset, fold-classified
 * root/delegated + active/revoked, as enumerated by `ListAssetGrants` (UI-3). Kept
 * in its own module so `types.ts` stays a thin aggregator, mirroring the Rust core's
 * module-per-concern discipline. A VIEW-only surface; issuing/revoking grants is cloud.
 */

import type {
  GrantView as PbGrantView,
  ListAssetGrantsResponse as PbListAssetGrantsResponse,
} from "./gen/kortecx/v1/gateway_pb.js";

/** One grant on an asset, fold-classified. */
export class GrantView {
  constructor(
    readonly grantor: string,
    readonly grantee: string,
    readonly actions: readonly string[],
    readonly runtimeScope: string,
    /** A root grant (from the asset owner), vs a delegated sub-grant. */
    readonly isRoot: boolean,
    /** An authorized revocation makes the grant inert in the fold. */
    readonly revoked: boolean,
  ) {}

  static fromProto(g: PbGrantView): GrantView {
    return new GrantView(g.grantor, g.grantee, g.actions, g.runtimeScope, g.isRoot, g.revoked);
  }

  /** A stable display status for the inspector. */
  get status(): "revoked" | "root" | "delegated" {
    if (this.revoked) {
      return "revoked";
    }
    return this.isRoot ? "root" : "delegated";
  }
}

/** Every grant on one asset, with the bound owner echoed (`""` if unbound). */
export class AssetGrants {
  constructor(
    readonly owner: string,
    readonly grants: readonly GrantView[],
  ) {}

  static fromProto(r: PbListAssetGrantsResponse): AssetGrants {
    return new AssetGrants(
      r.owner,
      r.grants.map((g) => GrantView.fromProto(g)),
    );
  }
}
