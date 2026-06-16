/**
 * Slice-B FuzzyDiscovery — an ADVISORY fuzzy-in / exact-out retrieval hit
 * (`FuzzyDiscovery`, D151). Kept in its own module so `types.ts` stays a thin
 * aggregator, mirroring the Rust core's module-per-concern discipline.
 *
 * SN-8 (load-bearing): `scoreBp` is DISPLAY-ONLY — never an identity input. The
 * result a downstream consumer trusts is the ordered `contentRef` SET; the caller
 * joins back to bytes with an EXACT `getContent` on the ref ("fuzzy in, exact
 * out"). The approximate ANN ranking never reaches identity.
 */

import type { FuzzyHit as PbFuzzyHit } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One advisory discovery hit: the content-addressed ref (hex) + a display score. */
export class FuzzyHit {
  constructor(
    /** The 32-byte content-addressed id (hex) — the EXACT-OUT join key. */
    readonly contentRef: string,
    /** Display-only similarity in basis points (0..=10000); NEVER identity (SN-8). */
    readonly scoreBp: number,
  ) {}

  static fromProto(h: PbFuzzyHit): FuzzyHit {
    return new FuzzyHit(encode(h.contentRef), h.scoreBp);
  }

  /** The similarity as a 0..1 fraction (display only). */
  get score(): number {
    return this.scoreBp / 10_000;
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return { content_ref: this.contentRef, score_bp: this.scoreBp };
  }
}
