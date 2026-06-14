/** PR-4.1 feedback view — pure, no server. Rating mapping + row hex-encoding. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { FeedbackRow, ratingFromProto, ratingToProto } from "../src/feedback.js";
import { FeedbackRating, FeedbackRowSchema } from "../src/gen/kortecx/v1/gateway_pb.js";

const fill = (v: number, n: number): Uint8Array => new Uint8Array(n).fill(v);

describe("rating mapping", () => {
  it("maps the string rating to/from the proto enum", () => {
    expect(ratingToProto("up")).toBe(FeedbackRating.UP);
    expect(ratingToProto("down")).toBe(FeedbackRating.DOWN);
    expect(ratingFromProto(FeedbackRating.UP)).toBe("up");
    expect(ratingFromProto(FeedbackRating.DOWN)).toBe("down");
    expect(ratingFromProto(FeedbackRating.UNSPECIFIED)).toBeNull();
  });
});

describe("FeedbackRow.fromProto", () => {
  it("hex-encodes ids + maps the rating, with a snake_case toJSON", () => {
    const r = create(FeedbackRowSchema, {
      feedbackId: fill(0xab, 16),
      rating: FeedbackRating.UP,
      messageId: "answer-9",
      instanceId: fill(0x05, 16),
      moteId: fill(0x28, 32),
      contentRef: fill(0x44, 32),
      comment: "great",
      recipeHandle: "kx/recipes/chat",
      modelId: "qwen3-4b",
      submittedUnixMs: 1234n,
      rowid: 7n,
    });
    const row = FeedbackRow.fromProto(r);
    expect(row.feedbackId).toBe("ab".repeat(16));
    expect(row.rating).toBe("up");
    expect(row.messageId).toBe("answer-9");
    expect(row.instanceId).toBe("05".repeat(16));
    expect(row.moteId).toBe("28".repeat(32));
    expect(row.contentRef).toBe("44".repeat(32));
    expect(row.submittedUnixMs).toBe(1234);
    expect(row.rowid).toBe(7);
    expect(row.toJSON()).toEqual({
      feedback_id: "ab".repeat(16),
      rating: "up",
      message_id: "answer-9",
      instance_id: "05".repeat(16),
      mote_id: "28".repeat(32),
      content_ref: "44".repeat(32),
      comment: "great",
      recipe_handle: "kx/recipes/chat",
      model_id: "qwen3-4b",
      submitted_unix_ms: 1234,
      rowid: 7,
    });
  });

  it("all-zero target ids (a local-only turn) render as empty strings", () => {
    const r = create(FeedbackRowSchema, {
      feedbackId: fill(0xab, 16),
      rating: FeedbackRating.DOWN,
      messageId: "local-1",
      instanceId: fill(0x00, 16),
      moteId: fill(0x00, 32),
      contentRef: fill(0x00, 32),
      rowid: 1n,
    });
    const row = FeedbackRow.fromProto(r);
    expect(row.rating).toBe("down");
    expect(row.instanceId).toBe("");
    expect(row.moteId).toBe("");
    expect(row.contentRef).toBe("");
  });
});
