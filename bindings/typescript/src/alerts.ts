/**
 * The operator alerts inbox view — one terminal-failure alert enumerated by
 * `ListAlerts` (W1a-2). A read-only projection of the journal's TERMINAL `Failed`
 * facts (dead-letters + worker-reported terminal failures) into the gateway's
 * rebuildable-to-empty `alerts.db` read-cache — DISPLAY/TRIAGE-READ ONLY, never
 * truth, never identity, never a digest input. Serve-path admission refusals
 * write nothing to the journal, so they are not in this inbox. Kept in its own
 * module (the telemetry.ts module-per-concern precedent).
 *
 * SN-8: `alertId`/`moteId` are server-derived; the SDK only *encodes* the bytes
 * to hex. The triage LIFECYCLE (acknowledge/resolve), the alert-rule engine, and
 * notifications are a Cloud capability (D156/D129) — there is no mutate method.
 */

import type { AlertSummary as PbAlertSummary } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One terminal-failure alert: the failed Mote + its watermark-attributed run
 *  (`instanceId` is `""` when unattributed — all-zero on the wire), the failure
 *  class, a display severity (`"error"` | `"refused"`), and the `Failed` fact's
 *  `seq` (the deep-link cursor + pagination). `alertId` is server-derived +
 *  re-fold-stable. */
export class AlertSummary {
  constructor(
    readonly alertId: string,
    readonly moteId: string,
    readonly instanceId: string,
    readonly reasonClass: string,
    readonly reasonCode: number,
    readonly severity: string,
    readonly seq: number,
    readonly createdUnixMs: number,
  ) {}

  static fromProto(a: PbAlertSummary): AlertSummary {
    return new AlertSummary(
      encode(a.alertId),
      encode(a.moteId),
      // All-zero (or empty) = unattributed → "" (the telemetry/global-tail convention).
      a.instanceId.some((b) => b !== 0) ? encode(a.instanceId) : "",
      a.reasonClass,
      a.reasonCode,
      a.severity,
      Number(a.seq),
      Number(a.createdUnixMs),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      alert_id: this.alertId,
      mote_id: this.moteId,
      instance_id: this.instanceId,
      reason_class: this.reasonClass,
      reason_code: this.reasonCode,
      severity: this.severity,
      seq: this.seq,
      created_unix_ms: this.createdUnixMs,
    };
  }
}

/** One page of {@link AlertSummary} (newest-first) plus the `hasMore` cursor flag. */
export interface AlertsPage {
  readonly alerts: AlertSummary[];
  readonly hasMore: boolean;
}
