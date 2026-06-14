/**
 * User feedback on an answer (PR-4.1) — a 👍/👎 rating + optional note the
 * gateway records into its rebuildable-to-empty `feedback.db` sidecar via
 * `SubmitFeedback`, read back by `ListFeedback`. AUDIT/DISPLAY ONLY: client-origin
 * product signal, never truth, never identity, never a digest input. Kept in its
 * own module (the telemetry.ts module-per-concern precedent).
 *
 * SN-8: the caller principal + the `feedbackId` are server-derived; the SDK only
 * *encodes* the bytes to hex and maps the rating enum.
 */

import type { FeedbackRow as PbFeedbackRow } from "./gen/kortecx/v1/gateway_pb.js";
import { FeedbackRating } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** A 👍/👎 rating as a stable string (the wire enum, lowercased). */
export type Rating = "up" | "down";

/** Map a {@link Rating} string to the proto enum int (UP=1, DOWN=2). */
export function ratingToProto(rating: Rating): FeedbackRating {
  return rating === "up" ? FeedbackRating.UP : FeedbackRating.DOWN;
}

/** Map the proto rating enum int back to a {@link Rating} (`null` if unset). */
export function ratingFromProto(r: FeedbackRating): Rating | null {
  if (r === FeedbackRating.UP) return "up";
  if (r === FeedbackRating.DOWN) return "down";
  return null;
}

/** The target + context the UI supplies when rating an answer. `messageId` is the
 *  stable per-answer key (required); the rest are advisory join/context fields
 *  (all-zero/absent is fine for a local-only turn with no run). */
export interface FeedbackInput {
  readonly rating: Rating;
  readonly messageId: string;
  readonly instanceId?: string;
  readonly moteId?: string;
  readonly contentRef?: string;
  readonly comment?: string;
  readonly recipeHandle?: string;
  readonly modelId?: string;
}

/** One recorded feedback row in a {@link FeedbackPage} (newest-first). `instanceId`
 *  is `""` when the turn had no run; `moteId`/`contentRef` are `""` when absent. */
export class FeedbackRow {
  constructor(
    readonly feedbackId: string,
    readonly rating: Rating | null,
    readonly messageId: string,
    readonly instanceId: string,
    readonly moteId: string,
    readonly contentRef: string,
    readonly comment: string,
    readonly recipeHandle: string,
    readonly modelId: string,
    readonly submittedUnixMs: number,
    readonly rowid: number,
  ) {}

  static fromProto(r: PbFeedbackRow): FeedbackRow {
    const nonEmpty = (b: Uint8Array): string => (b.some((x) => x !== 0) ? encode(b) : "");
    return new FeedbackRow(
      encode(r.feedbackId),
      ratingFromProto(r.rating),
      r.messageId,
      nonEmpty(r.instanceId),
      nonEmpty(r.moteId),
      nonEmpty(r.contentRef),
      r.comment,
      r.recipeHandle,
      r.modelId,
      Number(r.submittedUnixMs),
      Number(r.rowid),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      feedback_id: this.feedbackId,
      rating: this.rating,
      message_id: this.messageId,
      instance_id: this.instanceId,
      mote_id: this.moteId,
      content_ref: this.contentRef,
      comment: this.comment,
      recipe_handle: this.recipeHandle,
      model_id: this.modelId,
      submitted_unix_ms: this.submittedUnixMs,
      rowid: this.rowid,
    };
  }
}

/** One page of {@link FeedbackRow} (newest-first) plus the `hasMore` cursor flag. */
export interface FeedbackPage {
  readonly rows: FeedbackRow[];
  readonly hasMore: boolean;
}
