/**
 * The run-summary view — one registered run instance enumerated by `ListRuns`
 * (hex ids + the registered seq/wall-clock). Kept in its own module so `types.ts`
 * stays a thin aggregator, mirroring the Rust core's module-per-concern discipline.
 *
 * SN-8: every id is server-derived; the SDK only *encodes* the bytes to hex. The
 * `registeredUnixMs` is an audit-only wall-clock (off every hash) — a legitimate
 * "started at" for the UI, never identity.
 */

import type {
  GetRunInputsResponse as PbGetRunInputsResponse,
  RunSummary as PbRunSummary,
} from "./gen/kortecx/v1/gateway_pb.js";
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

/**
 * The captured `Invoke` args for a run (PR-D `GetRunInputs`) — the baseline a
 * client edits and re-invokes ("Re-run with changes"). `args` is decoded from the
 * opaque JSON object bytes the run was submitted with; `handle` is what
 * `getRecipeForm` needs to re-render the form (a durable run otherwise carries
 * only the fingerprint).
 *
 * SN-8 / off-digest: the args never become committed facts. A run with nothing
 * captured surfaces as `NotFound`; an old gateway as an `Unimplemented` rpc error.
 */
export class RunInputs {
  constructor(
    readonly instanceId: string,
    readonly recipeFingerprint: string,
    readonly handle: string,
    readonly args: Record<string, unknown>,
  ) {}

  static fromProto(r: PbGetRunInputsResponse): RunInputs {
    let args: Record<string, unknown> = {};
    if (r.args.length > 0) {
      try {
        const parsed: unknown = JSON.parse(new TextDecoder().decode(r.args));
        if (parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)) {
          args = parsed as Record<string, unknown>;
        }
      } catch {
        // A corrupt/non-JSON capture degrades to empty args (the parseLocalArgs
        // posture) rather than throwing inside the SDK — never fake, never crash.
        args = {};
      }
    }
    return new RunInputs(encode(r.instanceId), encode(r.recipeFingerprint), r.handle, args);
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      instance_id: this.instanceId,
      recipe_fingerprint: this.recipeFingerprint,
      handle: this.handle,
      args: this.args,
    };
  }
}
