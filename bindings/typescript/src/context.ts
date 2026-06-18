/**
 * PR-7 context-bundle views — a named, content-addressed collection a caller
 * attaches to a run (`invoke(handle, args, { context: [handle] })`) so a model
 * reasons over it. Kept in its own module so `types.ts` stays a thin aggregator
 * (the Rust core's module-per-concern discipline, GR3).
 *
 * SN-8: `bundleRef` is SERVER-DERIVED (blake3 over the manifest) — the client
 * names a handle, never an identity. The manifest lives in an off-journal
 * `bundles.db` sidecar (rebuildable-to-empty), scoped to the authoring party; a
 * not-found / not-owned bundle is UNIFORM (no cross-party existence oracle).
 */

import type { Id } from "./client.js";
import type {
  ContextBundle as PbContextBundle,
  ContextItem as PbContextItem,
  PutContextBundleResponse as PbPutContextBundleResponse,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One item to put in a context bundle: a label + a content-store ref. */
export interface ContextItemInput {
  /** Advisory label / context heading (display only). */
  name: string;
  /** A ref already in the content store (e.g. from {@link KxClientBase.putContent}). */
  contentRef: Id;
  /** Advisory mime (display / classify only). */
  mediaType?: string;
}

/** One item in a context bundle: an advisory label + a content-store ref. */
export class ContextBundleItem {
  constructor(
    readonly name: string,
    /** The 32-byte content-store ref, as 64 hex chars. */
    readonly contentRef: string,
    readonly mediaType: string,
  ) {}

  static fromProto(it: PbContextItem): ContextBundleItem {
    return new ContextBundleItem(it.name, encode(it.contentRef), it.mediaType);
  }

  toJSON() {
    return { name: this.name, content_ref: this.contentRef, media_type: this.mediaType };
  }
}

/** A context bundle's bound manifest (the governance / display view). */
export class ContextBundle {
  constructor(
    /** The server-derived manifest hash, as hex. */
    readonly bundleRef: string,
    readonly handle: string,
    readonly description: string,
    readonly items: ContextBundleItem[],
    readonly itemCount: number,
  ) {}

  static fromProto(b: PbContextBundle): ContextBundle {
    return new ContextBundle(
      encode(b.bundleRef),
      b.handle,
      b.description,
      b.items.map((it) => ContextBundleItem.fromProto(it)),
      b.itemCount,
    );
  }

  toJSON() {
    return {
      bundle_ref: this.bundleRef,
      handle: this.handle,
      description: this.description,
      item_count: this.itemCount,
      items: this.items.map((i) => i.toJSON()),
    };
  }
}

/** The outcome of a `PutContextBundle` upsert (server-derived ref + dedup flag). */
export class PutContextBundleResult {
  constructor(
    readonly bundleRef: string,
    readonly handle: string,
    readonly deduplicated: boolean,
  ) {}

  static fromProto(r: PbPutContextBundleResponse): PutContextBundleResult {
    return new PutContextBundleResult(encode(r.bundleRef), r.handle, r.deduplicated);
  }

  toJSON() {
    return { bundle_ref: this.bundleRef, handle: this.handle, deduplicated: this.deduplicated };
  }
}
