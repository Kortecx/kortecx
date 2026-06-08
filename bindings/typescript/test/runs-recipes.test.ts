/** UI-2 run-summary + recipe-form views — pure, no server. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
  GetRecipeFormResponseSchema,
  RecipeFormFieldSchema,
  RecipeParamType,
  RunSummarySchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";
import { RecipeForm, RecipeFormField, recipeParamTypeName } from "../src/recipes.js";
import { RunSummary } from "../src/runs.js";

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
    const resp = create(GetRecipeFormResponseSchema, { handle: "kx/recipes/fanout-demo" });
    const form = RecipeForm.fromProto(resp);
    expect(form.fields).toHaveLength(0);
  });
});
