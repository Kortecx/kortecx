/**
 * The run-summary view — one registered run instance enumerated by `ListRuns`
 * (hex ids + the registered seq/wall-clock). Kept in its own module so `types.ts`
 * stays a thin aggregator, mirroring the Rust core's module-per-concern discipline.
 *
 * SN-8: every id is server-derived; the SDK only *encodes* the bytes to hex. The
 * `registeredUnixMs` is an audit-only wall-clock (off every hash) — a legitimate
 * "started at" for the UI, never identity.
 */

import type { RunSummary as PbRunSummary } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One registered run instance: hex ids + the registered seq (pagination cursor)
 *  + the registered wall-clock (unix-ms; audit-only). */
export class RunSummary {
  constructor(
    readonly instanceId: string,
    readonly recipeFingerprint: string,
    readonly registeredSeq: number,
    readonly registeredUnixMs: number,
  ) {}

  static fromProto(r: PbRunSummary): RunSummary {
    return new RunSummary(
      encode(r.instanceId),
      encode(r.recipeFingerprint),
      Number(r.registeredSeq),
      Number(r.registeredUnixMs),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      instance_id: this.instanceId,
      recipe_fingerprint: this.recipeFingerprint,
      registered_seq: this.registeredSeq,
      registered_unix_ms: this.registeredUnixMs,
    };
  }
}

/** One page of {@link RunSummary} (newest-first) plus the `hasMore` cursor flag. */
export interface RunPage {
  readonly runs: RunSummary[];
  readonly hasMore: boolean;
}
