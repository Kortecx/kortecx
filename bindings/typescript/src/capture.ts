/**
 * The Morphic Data Engine capture view — one durably-captured ACTION record
 * enumerated by `ListCaptureRecords`. The serve-path action exhaust: a committed
 * Mote's join keys (`moteId` / `instanceId` / `resultRef` / `ndClass` / `seq`),
 * plus the ReAct `turn`/`branch` when the Mote is a ReAct turn. Join-key-only
 * (the privacy-safe ActionsOnly scope) — no payload/reasoning. Kept in its own
 * module (the runs.ts module-per-concern precedent).
 *
 * SN-8: ids are server-derived; the SDK only *encodes* the bytes to hex.
 */

import type { CaptureRecordSummary as PbCaptureRecordSummary } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One captured action: the committed Mote's join keys + (for a ReAct turn) the
 *  turn index/branch. `resultRef` IS the action's content address (the truth
 *  join key back to the journal). */
export class CaptureRecord {
  constructor(
    readonly moteId: string,
    readonly instanceId: string,
    readonly resultRef: string,
    readonly ndClass: string,
    readonly seq: number,
    readonly reactTurn: number | null,
    readonly reactBranch: string,
  ) {}

  static fromProto(r: PbCaptureRecordSummary): CaptureRecord {
    return new CaptureRecord(
      encode(r.moteId),
      encode(r.instanceId),
      encode(r.resultRef),
      r.ndClass,
      Number(r.seq),
      r.reactTurn === undefined ? null : r.reactTurn,
      r.reactBranch,
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      mote_id: this.moteId,
      instance_id: this.instanceId,
      result_ref: this.resultRef,
      nd_class: this.ndClass,
      seq: this.seq,
      react_turn: this.reactTurn,
      react_branch: this.reactBranch,
    };
  }
}

/** One page of {@link CaptureRecord} (newest-first) plus the `hasMore` cursor flag. */
export interface CaptureRecordPage {
  readonly records: CaptureRecord[];
  readonly hasMore: boolean;
}
