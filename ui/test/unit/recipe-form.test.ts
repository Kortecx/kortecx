import { RecipeForm, RecipeFormField } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { buildArgs, initialValues, validateField } from "../../src/lib/recipe-form";

const str = (name: string, required = true, maxLen: number | null = null) =>
  new RecipeFormField(name, "str", required, maxLen, []);
const int = (name: string) => new RecipeFormField(name, "int", true, null, []);
const bool = (name: string) => new RecipeFormField(name, "bool", true, null, []);
const enumf = (name: string, allowed: string[]) =>
  new RecipeFormField(name, "enum", true, null, allowed);

const form = (...fields: RecipeFormField[]) => new RecipeForm("kx/recipes/x", fields);

describe("initialValues", () => {
  it("defaults bool→false, enum→first allowed, others→empty", () => {
    const v = initialValues(form(str("topic"), bool("flag"), enumf("mode", ["a", "b"])));
    expect(v).toEqual({ topic: "", flag: "false", mode: "a" });
  });
});

describe("validateField", () => {
  it("required string rejects empty, accepts non-empty", () => {
    expect(validateField(str("t"), "")).toBe("required");
    expect(validateField(str("t"), "hi")).toBeNull();
  });

  it("optional string accepts empty", () => {
    expect(validateField(str("t", false), "")).toBeNull();
  });

  it("string maxLen is enforced", () => {
    expect(validateField(str("t", true, 3), "abcd")).toMatch(/at most 3/);
    expect(validateField(str("t", true, 3), "abc")).toBeNull();
  });

  it("int rejects non-integers (incl. floats) and accepts whole numbers", () => {
    expect(validateField(int("n"), "1.5")).toMatch(/whole number/);
    expect(validateField(int("n"), "abc")).toMatch(/whole number/);
    expect(validateField(int("n"), "-7")).toBeNull();
  });

  it("bool only accepts true/false", () => {
    expect(validateField(bool("b"), "true")).toBeNull();
    expect(validateField(bool("b"), "false")).toBeNull();
    expect(validateField(bool("b"), "maybe")).toMatch(/true or false/);
  });

  it("enum requires a permitted value", () => {
    expect(validateField(enumf("m", ["a", "b"]), "c")).toMatch(/one of: a, b/);
    expect(validateField(enumf("m", ["a", "b"]), "b")).toBeNull();
  });
});

describe("buildArgs", () => {
  it("coerces per type and omits blank optionals", () => {
    const f = form(str("topic"), int("n"), bool("flag"), str("note", false));
    const r = buildArgs(f, { topic: "hi", n: "42", flag: "true", note: "" });
    expect(r).toEqual({ ok: true, args: { topic: "hi", n: 42, flag: true } });
  });

  it("collects per-field errors and fails closed", () => {
    const f = form(str("topic"), int("n"));
    const r = buildArgs(f, { topic: "", n: "x" });
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.errors.topic).toBe("required");
      expect(r.errors.n).toMatch(/whole number/);
    }
  });

  it("an empty form builds empty args", () => {
    expect(buildArgs(form(), {})).toEqual({ ok: true, args: {} });
  });
});
