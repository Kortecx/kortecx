/**
 * The Batch A content views — a `PutContent` upload outcome and one
 * `GetContentBatch` item. Kept in its own module so `types.ts` stays a thin
 * aggregator (the Rust core's module-per-concern discipline, GR3).
 *
 * SN-8: `contentRef` is SERVER-DERIVED (blake3 over the payload) — the client
 * never names an identity. An upload is a CONTENT-STORE write, never a journal
 * write; `mediaType`/`filename` are advisory audit fields. A batch item whose
 * ref was unauthorized / missing / malformed comes back UNIFORMLY EMPTY
 * (`payload.length === 0 && fullSize === 0n`) — no existence oracle (D120.1).
 */

import type {
  ContentBatchItem as PbContentBatchItem,
  PutContentResponse as PbPutContentResponse,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** The outcome of a `PutContent` upload (server-derived ref + dedup flag). */
export class PutResult {
  constructor(
    /** The server-derived blake3 ref of the stored payload, as 64 hex chars. */
    readonly contentRef: string,
    /** Stored byte count. */
    readonly size: bigint,
    /** `true` iff an identical blob already existed (advisory display state). */
    readonly deduplicated: boolean,
  ) {}

  static fromProto(r: PbPutContentResponse): PutResult {
    return new PutResult(encode(r.contentRef), r.size, r.deduplicated);
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      content_ref: this.contentRef,
      size: Number(this.size),
      deduplicated: this.deduplicated,
    };
  }
}

/** One `GetContentBatch` item, in request order. */
export class ContentItem {
  constructor(
    /** The requested ref echoed back, as hex. */
    readonly contentRef: string,
    /** The payload bytes — EMPTY when unauthorized/missing/malformed (uniform). */
    readonly payload: Uint8Array,
    /** `true` iff `payload` was cut at the per-item clamp. */
    readonly truncated: boolean,
    /** The stored size — `0` when unauthorized/missing (uniform, honest). */
    readonly fullSize: bigint,
  ) {}

  static fromProto(i: PbContentBatchItem): ContentItem {
    return new ContentItem(encode(i.contentRef), i.payload, i.truncated, i.fullSize);
  }

  /** `true` iff the server returned the uniform empty item for this ref. */
  get missing(): boolean {
    return this.payload.length === 0 && this.fullSize === 0n;
  }

  /** The payload decoded as UTF-8 (best-effort) — for text content. */
  get text(): string {
    return new TextDecoder().decode(this.payload);
  }
}
