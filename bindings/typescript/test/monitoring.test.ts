/** Batch C monitoring views — pure, no server. Global event tail + telemetry. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { ndClassFromTag, wsAllDelta, wsAllUrl, wsUrl } from "../src/events.js";
import {
  CommittedDeltaSchema,
  EffectStagedDeltaSchema,
  FailedDeltaSchema,
  GlobalEventDeltaSchema,
  ListTelemetrySummaryResponseSchema,
  MoteTelemetryRowSchema,
  RepudiatedDeltaSchema,
  RunRegisteredDeltaSchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";
import { MoteTelemetryRow, TelemetrySummary } from "../src/telemetry.js";
import { GlobalDelta } from "../src/types.js";

const fill = (v: number, n: number): Uint8Array => new Uint8Array(n).fill(v);

// --- WS URL derivation (the global channel mirrors the per-run one) -----------

describe("wsAllUrl", () => {
  it("uses an explicit ws endpoint (trailing slash stripped), no instance param", () => {
    expect(wsAllUrl("http://127.0.0.1:50151", "ws://h:1/", 7n)).toBe(
      "ws://h:1/v1/events/all?since=7",
    );
  });

  it("derives ws/wss + the conventional port from the gRPC endpoint", () => {
    expect(wsAllUrl("http://127.0.0.1:50151", undefined, 0n)).toBe(
      "ws://127.0.0.1:50152/v1/events/all?since=0",
    );
    expect(wsAllUrl("https://gw.example.com:50151", undefined, 3n)).toBe(
      "wss://gw.example.com:50152/v1/events/all?since=3",
    );
  });

  it("the per-run wsUrl keeps its exact shape (the shared base derivation)", () => {
    expect(wsUrl("http://127.0.0.1:50151", undefined, "ab".repeat(16), 5n)).toBe(
      `ws://127.0.0.1:50152/v1/events?instance=${"ab".repeat(16)}&since=5`,
    );
  });
});

// --- the global WS JSON delta parser (all six tags, unknown-tolerant) ---------

describe("ndClassFromTag", () => {
  it("inverts the wire nd_class string tag to its discriminant; unknown ⇒ null (no fabricated 0)", () => {
    expect(ndClassFromTag("pure")).toBe(1);
    expect(ndClassFromTag("read_only_nondet")).toBe(2);
    expect(ndClassFromTag("world_mutating")).toBe(3);
    expect(ndClassFromTag("unspecified")).toBe(0);
    expect(ndClassFromTag(null)).toBeNull();
    expect(ndClassFromTag("future_tag")).toBeNull();
  });
});

describe("wsAllDelta", () => {
  it("parses run_registered with the camelCase view fields", () => {
    const d = wsAllDelta({
      type: "run_registered",
      seq: 1,
      instance_id: "11".repeat(16),
      recipe_fingerprint: "22".repeat(32),
      registered_unix_ms: 1234,
    });
    expect(d.kind).toBe("run_registered");
    expect(d.instanceId).toBe("11".repeat(16));
    expect(d.recipeFingerprint).toBe("22".repeat(32));
    expect(d.registeredUnixMs).toBe(1234);
    expect(d.moteId).toBeNull();
  });

  it("parses the four per-run kinds with instance_id attribution", () => {
    const committed = wsAllDelta({
      type: "committed",
      seq: 2,
      instance_id: "11".repeat(16),
      mote_id: "33".repeat(32),
      result_ref: "44".repeat(32),
      nd_class: "pure",
    });
    expect(committed.kind).toBe("committed");
    expect(committed.instanceId).toBe("11".repeat(16));
    expect(committed.moteId).toBe("33".repeat(32));
    expect(committed.resultRef).toBe("44".repeat(32));
    // The wire `nd_class` STRING tag is parsed back to its discriminant (the
    // GR16-caught gap: the committed arm used to DROP it → a null/0 export).
    expect(committed.ndClass).toBe(1); // "pure" → 1

    const failed = wsAllDelta({
      type: "failed",
      seq: 3,
      instance_id: "11".repeat(16),
      mote_id: "33".repeat(32),
      reason_class: 2,
    });
    expect(failed.kind).toBe("failed");
    expect(failed.reasonClass).toBe(2);

    const repudiated = wsAllDelta({
      type: "repudiated",
      seq: 4,
      instance_id: "11".repeat(16),
      target_mote_id: "55".repeat(32),
      target_committed_seq: 2,
    });
    expect(repudiated.kind).toBe("repudiated");
    expect(repudiated.targetMoteId).toBe("55".repeat(32));
    expect(repudiated.targetCommittedSeq).toBe(2);

    const staged = wsAllDelta({
      type: "effect_staged",
      seq: 5,
      instance_id: "11".repeat(16),
      mote_id: "33".repeat(32),
    });
    expect(staged.kind).toBe("effect_staged");
    expect(staged.moteId).toBe("33".repeat(32));
  });

  it("a pre-registration delta carries the honest empty instance_id", () => {
    const d = wsAllDelta({ type: "committed", seq: 1, instance_id: "", mote_id: "33".repeat(32) });
    expect(d.instanceId).toBe("");
  });

  it("an unknown future type parses to the unknown variant — never a throw", () => {
    const d = wsAllDelta({ type: "quantum_flux", seq: 9, instance_id: "11".repeat(16) });
    expect(d.kind).toBe("unknown");
    expect(d.seq).toBe(9);
    expect(d.instanceId).toBe("11".repeat(16));
    // No type tag at all is equally tolerated.
    expect(wsAllDelta({ seq: 1 }).kind).toBe("unknown");
    expect(wsAllDelta({ seq: 1 }).instanceId).toBe("");
  });
});

// --- the proto-side GlobalDelta view (the gRPC stream's mapper) ---------------

describe("GlobalDelta.fromProto", () => {
  it("maps run_registered + committed, with a snake_case toJSON", () => {
    const rr = create(GlobalEventDeltaSchema, {
      seq: 1n,
      instanceId: fill(0x11, 16),
      kind: {
        case: "runRegistered",
        value: create(RunRegisteredDeltaSchema, {
          recipeFingerprint: fill(0x22, 32),
          registeredUnixMs: 1234n,
        }),
      },
    });
    const reg = GlobalDelta.fromProto(rr);
    expect(reg.kind).toBe("run_registered");
    expect(reg.instanceId).toBe("11".repeat(16));
    expect(reg.recipeFingerprint).toBe("22".repeat(32));
    expect(reg.registeredUnixMs).toBe(1234);
    expect(reg.toJSON()).toEqual({
      seq: 1,
      kind: "run_registered",
      instance_id: "11".repeat(16),
      recipe_fingerprint: "22".repeat(32),
      registered_unix_ms: 1234,
    });

    const committed = create(GlobalEventDeltaSchema, {
      seq: 2n,
      instanceId: fill(0x11, 16),
      kind: {
        case: "committed",
        value: create(CommittedDeltaSchema, {
          moteId: fill(0x33, 32),
          resultRef: fill(0x44, 32),
          ndClass: 1,
        }),
      },
    });
    const c = GlobalDelta.fromProto(committed);
    expect(c.kind).toBe("committed");
    expect(c.moteId).toBe("33".repeat(32));
    expect(c.resultRef).toBe("44".repeat(32));
    expect(c.ndClass).toBe(1);
  });

  it("maps failed / repudiated / effect_staged", () => {
    const failed = create(GlobalEventDeltaSchema, {
      seq: 3n,
      instanceId: fill(0x11, 16),
      kind: {
        case: "failed",
        value: create(FailedDeltaSchema, { moteId: fill(0x33, 32), reasonClass: 2 }),
      },
    });
    expect(GlobalDelta.fromProto(failed).reasonClass).toBe(2);

    const repudiated = create(GlobalEventDeltaSchema, {
      seq: 4n,
      instanceId: fill(0x11, 16),
      kind: {
        case: "repudiated",
        value: create(RepudiatedDeltaSchema, {
          targetMoteId: fill(0x55, 32),
          targetCommittedSeq: 2n,
        }),
      },
    });
    const r = GlobalDelta.fromProto(repudiated);
    expect(r.targetMoteId).toBe("55".repeat(32));
    expect(r.targetCommittedSeq).toBe(2);

    const staged = create(GlobalEventDeltaSchema, {
      seq: 5n,
      instanceId: fill(0x11, 16),
      kind: {
        case: "effectStaged",
        value: create(EffectStagedDeltaSchema, { moteId: fill(0x33, 32) }),
      },
    });
    expect(GlobalDelta.fromProto(staged).kind).toBe("effect_staged");
  });

  it("no kind → unknown (never null, never a throw); empty instance_id is honest", () => {
    const d = GlobalDelta.fromProto(create(GlobalEventDeltaSchema, { seq: 9n }));
    expect(d.kind).toBe("unknown");
    expect(d.seq).toBe(9);
    expect(d.instanceId).toBe(""); // EMPTY pre-registration
    expect(d.toJSON()).toEqual({ seq: 9, kind: "unknown", instance_id: "" });
  });
});

// --- telemetry row mapping ------------------------------------------------------

describe("MoteTelemetryRow.fromProto", () => {
  it("hex-encodes ids + carries the exhaust fields, with a snake_case toJSON", () => {
    const r = create(MoteTelemetryRowSchema, {
      moteId: fill(0x28, 32),
      instanceId: fill(0x05, 16),
      wallClockMs: 42n,
      outputTokens: 128n,
      modelId: "qwen3-4b",
      toolId: "mcp-echo",
      startedUnixMs: 1234n,
      seq: 7n,
    });
    const row = MoteTelemetryRow.fromProto(r);
    expect(row.moteId).toBe("28".repeat(32));
    expect(row.instanceId).toBe("05".repeat(16));
    expect(row.wallClockMs).toBe(42);
    expect(row.inputTokens).toBeNull(); // NEVER set in OSS
    expect(row.outputTokens).toBe(128);
    expect(row.modelId).toBe("qwen3-4b");
    expect(row.toolId).toBe("mcp-echo");
    expect(row.startedUnixMs).toBe(1234);
    expect(row.seq).toBe(7);
    expect(row.toJSON()).toEqual({
      mote_id: "28".repeat(32),
      instance_id: "05".repeat(16),
      wall_clock_ms: 42,
      input_tokens: null,
      output_tokens: 128,
      model_id: "qwen3-4b",
      tool_id: "mcp-echo",
      started_unix_ms: 1234,
      seq: 7,
    });
  });

  it("absent optional tokens map to null; empty model/tool stay empty strings", () => {
    const r = create(MoteTelemetryRowSchema, {
      moteId: fill(0x01, 32),
      instanceId: fill(0x02, 16),
      wallClockMs: 1n,
      startedUnixMs: 1n,
      seq: 1n,
    });
    const row = MoteTelemetryRow.fromProto(r);
    expect(row.inputTokens).toBeNull();
    expect(row.outputTokens).toBeNull();
    expect(row.modelId).toBe("");
    expect(row.toolId).toBe("");
  });

  it("an all-zero instance_id (unattributed) renders as the empty string", () => {
    const r = create(MoteTelemetryRowSchema, {
      moteId: fill(0x01, 32),
      instanceId: fill(0x00, 16),
      wallClockMs: 1n,
      startedUnixMs: 1n,
      seq: 1n,
    });
    expect(MoteTelemetryRow.fromProto(r).instanceId).toBe("");
  });
});

describe("TelemetrySummary.fromProto (W1a-3)", () => {
  it("maps per-model rows + window totals, with a snake_case toJSON", () => {
    const resp = create(ListTelemetrySummaryResponseSchema, {
      rows: [
        { modelId: "model-a", count: 3n, totalOutputTokens: 60n, totalWallClockMs: 12n },
        { modelId: "model-b", count: 1n, totalOutputTokens: 5n, totalWallClockMs: 7n },
      ],
      totalMotes: 5n,
      totalOutputTokens: 65n,
    });
    const view = TelemetrySummary.fromProto(resp);
    expect(view.rows.map((r) => r.modelId)).toEqual(["model-a", "model-b"]);
    expect(view.rows[0]?.count).toBe(3);
    expect(view.rows[0]?.totalOutputTokens).toBe(60);
    expect(view.totalMotes).toBe(5);
    expect(view.totalOutputTokens).toBe(65);
    expect(view.toJSON()).toEqual({
      rows: [
        { model_id: "model-a", count: 3, total_output_tokens: 60, total_wall_clock_ms: 12 },
        { model_id: "model-b", count: 1, total_output_tokens: 5, total_wall_clock_ms: 7 },
      ],
      total_motes: 5,
      total_output_tokens: 65,
    });
  });

  it("an empty summary maps to empty rows + zero totals (no fabrication)", () => {
    const view = TelemetrySummary.fromProto(create(ListTelemetrySummaryResponseSchema, {}));
    expect(view.rows).toEqual([]);
    expect(view.totalMotes).toBe(0);
    expect(view.totalOutputTokens).toBe(0);
  });
});
