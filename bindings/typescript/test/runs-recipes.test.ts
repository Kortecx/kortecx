/** UI-2 run-summary + recipe-form + react/replan views — pure, no server. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { CaptureRecord } from "../src/capture.js";
import {
  CaptureRecordSummarySchema,
  GetRecipeFormResponseSchema,
  GetRunInputsResponseSchema,
  ReactTurnSummarySchema,
  RecipeFormFieldSchema,
  RecipeParamType,
  ReplanRoundSummarySchema,
  RunSummarySchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";
import { ReactTurn } from "../src/react.js";
import { RecipeForm, RecipeFormField, recipeParamTypeName } from "../src/recipes.js";
import { ReplanRound } from "../src/replan.js";
import { RunInputs, RunSummary } from "../src/runs.js";

const fill = (v: number, n: number): Uint8Array => new Uint8Array(n).fill(v);

describe("recipeParamTypeName", () => {
  it("maps every value domain and absorbs unknown", () => {
    expect(recipeParamTypeName(RecipeParamType.STR)).toBe("str");
    expect(recipeParamTypeName(RecipeParamType.INT)).toBe("int");
    expect(recipeParamTypeName(RecipeParamType.BOOL)).toBe("bool");
    expect(recipeParamTypeName(RecipeParamType.BYTES)).toBe("bytes");
    expect(recipeParamTypeName(RecipeParamType.ENUM)).toBe("enum");
    expect(recipeParamTypeName(RecipeParamType.UNSPECIFIED)).toBe("unspecified");
    expect(recipeParamTypeName(99)).toBe("unspecified");
  });
});

describe("RunSummary.fromProto", () => {
  it("hex-encodes ids + carries the seq/wall-clock, with a snake_case toJSON", () => {
    const r = create(RunSummarySchema, {
      instanceId: fill(0x11, 16),
      recipeFingerprint: fill(0x22, 32),
      registeredSeq: 7n,
      registeredUnixMs: 1234n,
    });
    const s = RunSummary.fromProto(r);
    expect(s.instanceId).toBe("11".repeat(16));
    expect(s.recipeFingerprint).toBe("22".repeat(32));
    expect(s.registeredSeq).toBe(7);
    expect(s.registeredUnixMs).toBe(1234);
    expect(s.toJSON()).toEqual({
      instance_id: "11".repeat(16),
      recipe_fingerprint: "22".repeat(32),
      registered_seq: 7,
      registered_unix_ms: 1234,
    });
  });
});

describe("RunInputs.fromProto", () => {
  const enc = (s: string): Uint8Array => new TextEncoder().encode(s);

  it("decodes the captured args JSON + handle, snake_case toJSON", () => {
    const r = create(GetRunInputsResponseSchema, {
      instanceId: fill(0x11, 16),
      recipeFingerprint: fill(0x22, 32),
      handle: "kx/recipes/echo",
      args: enc('{"topic":"hi","count":3}'),
    });
    const ri = RunInputs.fromProto(r);
    expect(ri.instanceId).toBe("11".repeat(16));
    expect(ri.recipeFingerprint).toBe("22".repeat(32));
    expect(ri.handle).toBe("kx/recipes/echo");
    expect(ri.args).toEqual({ topic: "hi", count: 3 });
    expect(ri.toJSON()).toEqual({
      instance_id: "11".repeat(16),
      recipe_fingerprint: "22".repeat(32),
      handle: "kx/recipes/echo",
      args: { topic: "hi", count: 3 },
    });
  });

  it("treats empty/non-object/malformed args as {} (never throws)", () => {
    const empty = RunInputs.fromProto(
      create(GetRunInputsResponseSchema, { handle: "h", args: new Uint8Array() }),
    );
    expect(empty.args).toEqual({});
    const arr = RunInputs.fromProto(
      create(GetRunInputsResponseSchema, { handle: "h", args: enc("[1,2,3]") }),
    );
    expect(arr.args).toEqual({});
    // A corrupt/non-JSON capture degrades to {} rather than throwing.
    const bad = RunInputs.fromProto(
      create(GetRunInputsResponseSchema, { handle: "h", args: enc("not json{") }),
    );
    expect(bad.args).toEqual({});
  });
});

describe("RecipeFormField.fromProto", () => {
  it("maps a typed STR field with maxLen", () => {
    const f = create(RecipeFormFieldSchema, {
      name: "topic",
      type: RecipeParamType.STR,
      required: true,
      maxLen: 4096n,
      allowed: [],
    });
    const field = RecipeFormField.fromProto(f);
    expect(field.name).toBe("topic");
    expect(field.type).toBe("str");
    expect(field.required).toBe(true);
    expect(field.maxLen).toBe(4096);
    expect(field.allowed).toEqual([]);
  });

  it("maps an ENUM field with allowed values + a null maxLen", () => {
    const f = create(RecipeFormFieldSchema, {
      name: "mode",
      type: RecipeParamType.ENUM,
      required: true,
      allowed: ["fast", "slow"],
    });
    const field = RecipeFormField.fromProto(f);
    expect(field.type).toBe("enum");
    expect(field.maxLen).toBeNull();
    expect(field.allowed).toEqual(["fast", "slow"]);
  });
});

describe("ReactTurn.fromProto", () => {
  it("hex-encodes ids + carries the branch/tool/caps, with a snake_case toJSON", () => {
    const t = create(ReactTurnSummarySchema, {
      turn: 2,
      turnMoteId: fill(0x28, 32),
      instanceId: fill(0x05, 16),
      modelId: "react-v1",
      branch: "tool",
      toolId: "mcp-echo",
      toolVersion: "1",
      maxTurns: 8,
      maxToolCalls: 6,
      seq: 42n,
      stepSalt: fill(0x5a, 32), // PR-R1: the chain key
    });
    const r = ReactTurn.fromProto(t);
    expect(r.turn).toBe(2);
    expect(r.turnMoteId).toBe("28".repeat(32));
    expect(r.instanceId).toBe("05".repeat(16));
    expect(r.branch).toBe("tool");
    expect(r.toolId).toBe("mcp-echo");
    expect(r.maxToolCalls).toBe(6);
    expect(r.seq).toBe(42);
    expect(r.stepSalt).toBe("5a".repeat(32));
    expect(r.toJSON()).toEqual({
      turn: 2,
      turn_mote_id: "28".repeat(32),
      instance_id: "05".repeat(16),
      model_id: "react-v1",
      branch: "tool",
      tool_id: "mcp-echo",
      tool_version: "1",
      max_turns: 8,
      max_tool_calls: 6,
      seq: 42,
      rejection_reason: "",
      step_salt: "5a".repeat(32),
    });
  });

  it("carries an answer branch with empty tool fields", () => {
    const t = create(ReactTurnSummarySchema, {
      turn: 0,
      turnMoteId: fill(0x01, 32),
      instanceId: fill(0x02, 16),
      modelId: "m",
      branch: "answer",
      maxTurns: 8,
      maxToolCalls: 6,
      seq: 3n,
    });
    const r = ReactTurn.fromProto(t);
    expect(r.branch).toBe("answer");
    expect(r.toolId).toBe("");
    expect(r.toolVersion).toBe("");
  });
});

describe("ReplanRound.fromProto", () => {
  it("hex-encodes the shaper + failed steps, with a snake_case toJSON", () => {
    const r = create(ReplanRoundSummarySchema, {
      round: 1,
      shaperMoteId: fill(0x1e, 32),
      modelId: "plan-v1",
      failedStepIds: [fill(0x1f, 32), fill(0x20, 32)],
      escalated: false,
      seq: 9n,
    });
    const round = ReplanRound.fromProto(r);
    expect(round.round).toBe(1);
    expect(round.shaperMoteId).toBe("1e".repeat(32));
    expect(round.failedStepIds).toEqual(["1f".repeat(32), "20".repeat(32)]);
    expect(round.escalated).toBe(false);
    expect(round.seq).toBe(9);
    expect(round.toJSON()).toEqual({
      round: 1,
      shaper_mote_id: "1e".repeat(32),
      model_id: "plan-v1",
      failed_step_ids: ["1f".repeat(32), "20".repeat(32)],
      escalated: false,
      seq: 9,
    });
  });
});

describe("CaptureRecord.fromProto", () => {
  it("hex-encodes the action join keys + the react join, with a snake_case toJSON", () => {
    const r = create(CaptureRecordSummarySchema, {
      moteId: fill(0x28, 32),
      instanceId: fill(0x05, 16),
      resultRef: fill(0x30, 32),
      ndClass: "read_only_nondet",
      seq: 7n,
      reactTurn: 2,
      reactBranch: "tool",
    });
    const rec = CaptureRecord.fromProto(r);
    expect(rec.moteId).toBe("28".repeat(32));
    expect(rec.instanceId).toBe("05".repeat(16));
    expect(rec.resultRef).toBe("30".repeat(32));
    expect(rec.ndClass).toBe("read_only_nondet");
    expect(rec.seq).toBe(7);
    expect(rec.reactTurn).toBe(2);
    expect(rec.reactBranch).toBe("tool");
    expect(rec.toJSON()).toEqual({
      mote_id: "28".repeat(32),
      instance_id: "05".repeat(16),
      result_ref: "30".repeat(32),
      nd_class: "read_only_nondet",
      seq: 7,
      react_turn: 2,
      react_branch: "tool",
    });
  });

  it("maps a non-react action with a null react_turn", () => {
    const r = create(CaptureRecordSummarySchema, {
      moteId: fill(0x01, 32),
      instanceId: fill(0x02, 16),
      resultRef: fill(0x03, 32),
      ndClass: "pure",
      seq: 1n,
    });
    const rec = CaptureRecord.fromProto(r);
    expect(rec.reactTurn).toBeNull();
    expect(rec.reactBranch).toBe("");
  });
});

describe("RecipeForm.fromProto", () => {
  it("wraps the handle + ordered fields", () => {
    const resp = create(GetRecipeFormResponseSchema, {
      handle: "kx/recipes/echo",
      fields: [
        { name: "topic", type: RecipeParamType.STR, required: true, maxLen: 4096n, allowed: [] },
      ],
    });
    const form = RecipeForm.fromProto(resp);
    expect(form.handle).toBe("kx/recipes/echo");
    expect(form.fields).toHaveLength(1);
    expect(form.fields[0]?.name).toBe("topic");
  });

  it("an empty form (no free-params) is a valid form", () => {
    const resp = create(GetRecipeFormResponseSchema, { handle: "kx/recipes/passthrough-dag" });
    const form = RecipeForm.fromProto(resp);
    expect(form.fields).toHaveLength(0);
  });
});
