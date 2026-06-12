/** Batch B mote-detail view — pure, no server. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { MoteConfigEntrySchema, MoteDetailSchema } from "../src/gen/kortecx/v1/gateway_pb.js";
import { MoteConfigItem, MoteDetail, effectPatternName, ndClassName } from "../src/motes.js";

const fill = (v: number, n: number): Uint8Array => new Uint8Array(n).fill(v);

describe("MoteDetail.fromProto", () => {
  it("hex-encodes ids, maps every field, snake_case toJSON (CLI parity)", () => {
    const d = create(MoteDetailSchema, {
      moteId: fill(0xa1, 32),
      moteDefHash: fill(0xb2, 32),
      defFound: true,
      stepKind: "model",
      modelId: "qwen3",
      prompt: "say hi",
      promptTruncated: false,
      configSubset: [
        create(MoteConfigEntrySchema, {
          key: "temperature",
          value: new Uint8Array([0x30]),
          truncated: false,
          fullLen: 1n,
        }),
      ],
      toolContract: { echo: "1" },
      logicRef: fill(0x07, 32),
      ndClass: 1,
      effectPattern: 1,
      criticFor: fill(0x03, 32),
      isTopologyShaper: false,
      schemaVersion: 5,
    });
    const detail = MoteDetail.fromProto(d);
    expect(detail.moteId).toBe("a1".repeat(32));
    expect(detail.moteDefHash).toBe("b2".repeat(32));
    expect(detail.defFound).toBe(true);
    expect(detail.stepKind).toBe("model");
    expect(detail.ndClassName).toBe("PURE");
    expect(detail.effectPatternName).toBe("IdempotentByConstruction");
    expect(detail.criticFor).toBe("03".repeat(32));
    expect(detail.toolContract).toEqual({ echo: "1" });
    expect(detail.toJSON()).toEqual({
      mote_id: "a1".repeat(32),
      mote_def_hash: "b2".repeat(32),
      def_found: true,
      step_kind: "model",
      model_id: "qwen3",
      prompt: "say hi",
      prompt_truncated: false,
      config_subset: [{ key: "temperature", value_hex: "30", truncated: false, full_len: 1 }],
      tool_contract: { echo: "1" },
      logic_ref: "07".repeat(32),
      nd_class: "PURE",
      effect_pattern: "IdempotentByConstruction",
      critic_for: "03".repeat(32),
      is_topology_shaper: false,
      schema_version: 5,
    });
  });

  it("renders the honest empty (def_found=false, empty hash, null critic_for)", () => {
    const d = create(MoteDetailSchema, { moteId: fill(0xa1, 32), defFound: false });
    const detail = MoteDetail.fromProto(d);
    expect(detail.defFound).toBe(false);
    expect(detail.moteDefHash).toBe("");
    expect(detail.criticFor).toBeUndefined();
    expect(detail.toJSON().critic_for).toBeNull();
  });
});

describe("display-name maps", () => {
  it("covers the closed vocabularies + UNKNOWN fallbacks", () => {
    expect(ndClassName(1)).toBe("PURE");
    expect(ndClassName(2)).toBe("READ_ONLY_NONDET");
    expect(ndClassName(3)).toBe("WORLD_MUTATING");
    expect(ndClassName(0)).toBe("UNKNOWN");
    expect(effectPatternName(1)).toBe("IdempotentByConstruction");
    expect(effectPatternName(2)).toBe("StageThenCommit");
    expect(effectPatternName(3)).toBe("ValidateThenCommit");
    expect(effectPatternName(99)).toBe("UNKNOWN");
  });
});

describe("MoteConfigItem", () => {
  it("keeps honest truncation lengths", () => {
    const e = create(MoteConfigEntrySchema, {
      key: "blob",
      value: fill(0x61, 8),
      truncated: true,
      fullLen: 5000n,
    });
    const item = MoteConfigItem.fromProto(e);
    expect(item.truncated).toBe(true);
    expect(item.fullLen).toBe(5000);
    expect(item.toJSON().value_hex).toBe("61".repeat(8));
  });
});
