/**
 * The HITL pre-action approval views (D114) — the operator control plane over pending
 * world-mutating action approvals (`ListPendingApprovals` / `GrantApproval` /
 * `DenyApproval`). Grant/deny are OPERATOR decisions over a server-derived
 * `requestId` — they release/reject a STAGED action, never mint a client warrant
 * (SN-8). Kept in its own module (the `triggers.ts`/`secrets.ts` module-per-concern
 * precedent).
 */

import type { PendingApproval as PbPendingApproval } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One world-mutating action withheld awaiting an operator decision (display-only). */
export class PendingApprovalRow {
  constructor(
    readonly requestId: string,
    readonly instanceId: string,
    readonly moteId: string,
    readonly toolId: string,
    readonly toolVersion: string,
    readonly intent: string,
    readonly deadlineUnixMs: number,
    readonly createdUnixMs: number,
  ) {}

  static fromProto(a: PbPendingApproval): PendingApprovalRow {
    return new PendingApprovalRow(
      encode(a.requestId),
      encode(a.instanceId),
      encode(a.moteId),
      a.toolId,
      a.toolVersion,
      a.intent,
      Number(a.deadlineUnixMs),
      Number(a.createdUnixMs),
    );
  }

  toJSON() {
    return {
      request_id: this.requestId,
      instance_id: this.instanceId,
      mote_id: this.moteId,
      tool_id: this.toolId,
      tool_version: this.toolVersion,
      intent: this.intent,
      deadline_unix_ms: this.deadlineUnixMs,
      created_unix_ms: this.createdUnixMs,
    };
  }
}

/** A page of pending approvals. */
export interface PendingApprovalsPage {
  readonly approvals: readonly PendingApprovalRow[];
}
