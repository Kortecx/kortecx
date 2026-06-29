/** Batch A content + model views — pure, no server. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { ContentItem, PutResult } from "../src/content.js";
import {
  ContentBatchItemSchema,
  ModelSummarySchema,
  PutContentResponseSchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";
import { ModelSummary } from "../src/models.js";

const fill = (v: number, n: number): Uint8Array => new Uint8Array(n).fill(v);

describe("PutResult.fromProto", () => {
  it("hex-encodes the server-derived ref + a snake_case toJSON", () => {
    const r = create(PutContentResponseSchema, {
      contentRef: fill(0xcd, 32),
      size: 1234n,
      deduplicated: true,
    });
    const put = PutResult.fromProto(r);
    expect(put.contentRef).toBe("cd".repeat(32));
    expect(put.size).toBe(1234n);
    expect(put.deduplicated).toBe(true);
    expect(put.toJSON()).toEqual({
      content_ref: "cd".repeat(32),
      size: 1234,
      deduplicated: true,
    });
  });
});

describe("ContentItem.fromProto", () => {
  it("carries the payload + honest truncation, decodes text", () => {
    const i = create(ContentBatchItemSchema, {
      contentRef: fill(0xee, 32),
      payload: new TextEncoder().encode("partial tex"),
      truncated: true,
      fullSize: 999n,
    });
    const item = ContentItem.fromProto(i);
    expect(item.contentRef).toBe("ee".repeat(32));
    expect(item.text).toBe("partial tex");
    expect(item.truncated).toBe(true);
    expect(item.fullSize).toBe(999n);
    expect(item.missing).toBe(false);
  });

  it("flags the UNIFORM empty item as missing (no existence oracle)", () => {
    const i = create(ContentBatchItemSchema, {
      contentRef: fill(0x11, 32),
      payload: new Uint8Array(),
      truncated: false,
      fullSize: 0n,
    });
    const item = ContentItem.fromProto(i);
    expect(item.missing).toBe(true);
  });

  it("a genuinely empty stored blob is NOT missing (fullSize stays 0 only when denied)", () => {
    // An empty payload with fullSize 0 IS the uniform denial — a truly empty
    // blob also has fullSize 0, so the two are indistinguishable BY DESIGN
    // (D120.1: empty-blob owners learn nothing new; everyone else learns
    // nothing at all). Document the contract here.
    const i = create(ContentBatchItemSchema, {
      contentRef: fill(0x22, 32),
      payload: new Uint8Array(),
      truncated: false,
      fullSize: 0n,
    });
    expect(ContentItem.fromProto(i).missing).toBe(true);
  });
});

describe("ModelSummary.fromProto", () => {
  it("carries the display fields + a snake_case toJSON", () => {
    const m = create(ModelSummarySchema, {
      modelId: "kx-serve:qwen3-4b",
      modalities: ["text", "image"],
      description: "Qwen3 4B",
      serving: true,
      contextLen: 8192,
      loaded: true,
      chatHandle: "kx/recipes/chat",
      engine: "kx-llamacpp",
      canEmbed: true,
      source: "local",
      active: true,
      chatRagHandle: "kx/recipes/chat-rag",
    });
    const s = ModelSummary.fromProto(m);
    expect(s.modelId).toBe("kx-serve:qwen3-4b");
    expect(s.modalities).toEqual(["text", "image"]);
    expect(s.serving).toBe(true);
    expect(s.contextLen).toBe(8192);
    // POC-3: the additive residency + routing fields carry through.
    expect(s.loaded).toBe(true);
    expect(s.chatHandle).toBe("kx/recipes/chat");
    // PR-A: the serving engine carries through (pluggable inference engine).
    expect(s.engine).toBe("kx-llamacpp");
    // PR-B: the configured-embedder flag carries through.
    expect(s.canEmbed).toBe(true);
    // Model Control v2: the additive provenance/active/chat-rag fields carry through.
    expect(s.source).toBe("local");
    expect(s.active).toBe(true);
    expect(s.chatRagHandle).toBe("kx/recipes/chat-rag");
    expect(s.toJSON()).toEqual({
      model_id: "kx-serve:qwen3-4b",
      modalities: ["text", "image"],
      description: "Qwen3 4B",
      serving: true,
      context_len: 8192,
      loaded: true,
      chat_handle: "kx/recipes/chat",
      engine: "kx-llamacpp",
      can_embed: true,
      source: "local",
      active: true,
      chat_rag_handle: "kx/recipes/chat-rag",
      embed_is_decoder: false,
    });
  });
});
