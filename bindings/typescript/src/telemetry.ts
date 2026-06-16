/**
 * The mote execution-telemetry view — one host-measured exhaust row enumerated
 * by `ListMoteTelemetry` (Batch C): wall-clock, model usage, the fired tool,
 * keyed by the Committed fact's `seq`. Lives in the gateway's rebuildable-to-empty
 * `telemetry.db` sidecar — AUDIT/DISPLAY ONLY, never truth, never identity, never
 * a digest input. Kept in its own module (the runs.ts module-per-concern
 * precedent).
 *
 * SN-8: ids are server-derived; the SDK only *encodes* the bytes to hex.
 */

import type {
  ListTelemetrySummaryResponse as PbListTelemetrySummaryResponse,
  ModelTokenRollup as PbModelTokenRollup,
  MoteTelemetryRow as PbMoteTelemetryRow,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One execution-telemetry row: the executed Mote + its watermark-attributed run
 *  (`instanceId` is `""` when unattributed — all-zero on the wire), host wall
 *  time, model/tool usage, and the Committed `seq` (the pagination cursor).
 *  `inputTokens` is NEVER set in OSS (the frozen backend seam reports no input
 *  count); `outputTokens` is set only for model motes on an inference build. */
export class MoteTelemetryRow {
  constructor(
    readonly moteId: string,
    readonly instanceId: string,
    readonly wallClockMs: number,
    readonly inputTokens: number | null,
    readonly outputTokens: number | null,
    readonly modelId: string,
    readonly toolId: string,
    readonly startedUnixMs: number,
    readonly seq: number,
  ) {}

  static fromProto(r: PbMoteTelemetryRow): MoteTelemetryRow {
    return new MoteTelemetryRow(
      encode(r.moteId),
      // All-zero (or empty) = unattributed → "" (the global-tail convention).
      r.instanceId.some((b) => b !== 0) ? encode(r.instanceId) : "",
      Number(r.wallClockMs),
      r.inputTokens === undefined ? null : Number(r.inputTokens),
      r.outputTokens === undefined ? null : Number(r.outputTokens),
      r.modelId,
      r.toolId,
      Number(r.startedUnixMs),
      Number(r.seq),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      mote_id: this.moteId,
      instance_id: this.instanceId,
      wall_clock_ms: this.wallClockMs,
      input_tokens: this.inputTokens,
      output_tokens: this.outputTokens,
      model_id: this.modelId,
      tool_id: this.toolId,
      started_unix_ms: this.startedUnixMs,
      seq: this.seq,
    };
  }
}

/** One page of {@link MoteTelemetryRow} (newest-first) plus the `hasMore` cursor flag. */
export interface MoteTelemetryPage {
  readonly rows: MoteTelemetryRow[];
  readonly hasMore: boolean;
}

/** One model's exact, cross-page token-economy rollup (`ListTelemetrySummary`,
 *  W1a-3): output tokens + wall-clock summed over every committed mote that ran
 *  `modelId`. Token-only — no cost/$ (billing is CLOUD). */
export class ModelTokenRollup {
  constructor(
    readonly modelId: string,
    readonly count: number,
    readonly totalOutputTokens: number,
    readonly totalWallClockMs: number,
  ) {}

  static fromProto(r: PbModelTokenRollup): ModelTokenRollup {
    return new ModelTokenRollup(
      r.modelId,
      Number(r.count),
      Number(r.totalOutputTokens),
      Number(r.totalWallClockMs),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      model_id: this.modelId,
      count: this.count,
      total_output_tokens: this.totalOutputTokens,
      total_wall_clock_ms: this.totalWallClockMs,
    };
  }
}

/** The per-model token rollup (descending output tokens) + the window-wide
 *  honest totals across ALL joined motes in scope (model + non-model). */
export class TelemetrySummary {
  constructor(
    readonly rows: ModelTokenRollup[],
    readonly totalMotes: number,
    readonly totalOutputTokens: number,
  ) {}

  static fromProto(resp: PbListTelemetrySummaryResponse): TelemetrySummary {
    return new TelemetrySummary(
      resp.rows.map((r) => ModelTokenRollup.fromProto(r)),
      Number(resp.totalMotes),
      Number(resp.totalOutputTokens),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      rows: this.rows.map((r) => r.toJSON()),
      total_motes: this.totalMotes,
      total_output_tokens: this.totalOutputTokens,
    };
  }
}
