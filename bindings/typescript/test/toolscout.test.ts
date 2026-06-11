/** W1.A5 toolscout views — pure, no server. Advisory/display-only (SN-8). */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
  KeywordSetSchema,
  ListToolManifestsResponseSchema,
  LowerVerdict,
  ManifestScoreSchema,
  ScoreTaskBundleRequestSchema,
  ScoreTaskBundleResponseSchema,
  ToolManifestSchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";
import { decodeFixed } from "../src/hexids.js";
import {
  BundleScore,
  KeywordSet,
  ManifestScore,
  ToolManifest,
  bundleSpecToProto,
  lowerVerdictName,
} from "../src/toolscout.js";

/** A deterministic 32-byte fingerprint (00 01 02 … 1f) and its hex form. */
const FP = Uint8Array.from({ length: 32 }, (_, i) => i);
const FP_HEX = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

describe("lowerVerdictName", () => {
  it("maps every wire value to the stable union (unknown absorbs the rest)", () => {
    expect(lowerVerdictName(LowerVerdict.UNAVAILABLE)).toBe("unavailable");
    expect(lowerVerdictName(LowerVerdict.WOULD_LOWER)).toBe("would-lower");
    expect(lowerVerdictName(LowerVerdict.REFUSED)).toBe("refused");
    expect(lowerVerdictName(LowerVerdict.UNSPECIFIED)).toBe("unknown");
    expect(lowerVerdictName(99)).toBe("unknown");
  });
});

describe("ToolManifest.fromProto", () => {
  it("maps the manifest (hash → 64-char lowercase hex, per-lang keywords)", () => {
    const r = create(ListToolManifestsResponseSchema, {
      manifests: [
        create(ToolManifestSchema, {
          toolId: "mcp-echo",
          toolVersion: "1",
          description: "echoes its input back",
          keywords: [create(KeywordSetSchema, { lang: "en", words: ["echo", "repeat"] })],
          fingerprintHash: FP,
          kind: "Mcp",
        }),
      ],
    });
    const manifests = r.manifests.map((m) => ToolManifest.fromProto(m));
    expect(manifests).toHaveLength(1);
    const m = manifests[0];
    expect(m).toBeInstanceOf(ToolManifest);
    expect(m?.toolId).toBe("mcp-echo");
    expect(m?.toolVersion).toBe("1");
    expect(m?.kind).toBe("Mcp");
    expect(m?.fingerprintHash).toBe(FP_HEX);
    expect(m?.fingerprintHash).toHaveLength(64);
    expect(m?.keywords[0]).toBeInstanceOf(KeywordSet);
    expect(m?.keywords[0]?.lang).toBe("en");
    expect(m?.keywords[0]?.words).toEqual(["echo", "repeat"]);
    // The stable snake_case serialization for UIs/logs.
    expect(m?.toJSON()).toEqual({
      tool_id: "mcp-echo",
      tool_version: "1",
      description: "echoes its input back",
      keywords: [{ lang: "en", words: ["echo", "repeat"] }],
      fingerprint_hash: FP_HEX,
      kind: "Mcp",
    });
  });
});

describe("BundleScore.fromProto", () => {
  it("maps a representative response (verdict enum → union, hashes → hex)", () => {
    const r = create(ScoreTaskBundleResponseSchema, {
      bundleFingerprint: FP,
      ranked: [
        create(ManifestScoreSchema, {
          toolId: "mcp-echo",
          toolVersion: "1",
          scoreBp: 10000,
          fingerprintHash: FP,
        }),
        create(ManifestScoreSchema, {
          toolId: "other",
          toolVersion: "2",
          scoreBp: 4200,
          fingerprintHash: FP,
        }),
      ],
      verdict: LowerVerdict.WOULD_LOWER,
      verdictDetail: "the grant gate passed",
    });
    const b = BundleScore.fromProto(r);
    expect(b).toBeInstanceOf(BundleScore);
    expect(b.bundleFingerprint).toBe(FP_HEX);
    expect(b.verdict).toBe("would-lower");
    expect(b.verdictDetail).toBe("the grant gate passed");
    expect(b.ranked).toHaveLength(2);
    expect(b.ranked[0]).toBeInstanceOf(ManifestScore);
    expect(b.ranked[0]?.scoreBp).toBe(10000);
    expect(b.ranked[1]?.scoreBp).toBe(4200);
    expect(b.ranked[0]?.fingerprintHash).toBe(FP_HEX);
  });

  it("maps a refused / unavailable verdict", () => {
    const refused = BundleScore.fromProto(
      create(ScoreTaskBundleResponseSchema, {
        verdict: LowerVerdict.REFUSED,
        verdictDetail: "tool not granted",
      }),
    );
    expect(refused.verdict).toBe("refused");
    expect(refused.ranked).toEqual([]);
    const unavailable = BundleScore.fromProto(
      create(ScoreTaskBundleResponseSchema, { verdict: LowerVerdict.UNAVAILABLE }),
    );
    expect(unavailable.verdict).toBe("unavailable");
  });
});

describe("bundleSpecToProto", () => {
  it("applies the defaults (empty tags/description/keywords, threshold 0)", () => {
    const init = bundleSpecToProto({
      intent: "echo a greeting",
      tools: [{ toolId: "mcp-echo", toolVersion: "1" }],
    });
    expect(init).toEqual({
      intent: "echo a greeting",
      languageTags: [],
      toolSequence: [{ toolId: "mcp-echo", toolVersion: "1", description: "", keywords: [] }],
      toleranceThresholdBp: 0,
    });
    // The init shape materializes into a valid wire request.
    const msg = create(ScoreTaskBundleRequestSchema, init);
    expect(msg.intent).toBe("echo a greeting");
    expect(msg.toleranceThresholdBp).toBe(0);
    expect(msg.toolSequence[0]?.toolId).toBe("mcp-echo");
  });

  it("carries explicit advisory metadata through to the wire", () => {
    const msg = create(
      ScoreTaskBundleRequestSchema,
      bundleSpecToProto({
        intent: "translate then echo",
        languageTags: ["en", "hi"],
        tools: [
          {
            toolId: "mcp-echo",
            toolVersion: "1",
            description: "echoes",
            keywords: [{ lang: "en", words: ["echo"] }],
          },
        ],
        toleranceThresholdBp: 7500,
      }),
    );
    expect(msg.languageTags).toEqual(["en", "hi"]);
    expect(msg.toleranceThresholdBp).toBe(7500);
    expect(msg.toolSequence[0]?.description).toBe("echoes");
    expect(msg.toolSequence[0]?.keywords[0]?.lang).toBe("en");
    expect(msg.toolSequence[0]?.keywords[0]?.words).toEqual(["echo"]);
  });
});

describe("round-trip", () => {
  it("hex fingerprints decode back to the server bytes", () => {
    const b = BundleScore.fromProto(
      create(ScoreTaskBundleResponseSchema, {
        bundleFingerprint: FP,
        verdict: LowerVerdict.WOULD_LOWER,
      }),
    );
    expect(decodeFixed(b.bundleFingerprint, 32)).toEqual(FP);
  });
});
