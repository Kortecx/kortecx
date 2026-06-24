/**
 * POC-5d: the App `input_schema` → RecipeForm mapping (the run drawer's typed
 * fields). Pure + total; unknown/missing ⇒ null (the App runs with no inputs).
 */

import { describe, expect, it } from "vitest";
import { appInputForm } from "../../src/lib/app-input-schema";

describe("appInputForm", () => {
  it("maps the {fields:[{name,type,required}]} subset onto a RecipeForm", () => {
    const form = appInputForm("apps/local/echo", {
      fields: [
        { name: "word", type: "str", required: true },
        { name: "count", type: "int" },
        { name: "mode", type: "enum", allowed: ["a", "b"] },
      ],
    });
    expect(form).not.toBeNull();
    expect(form?.handle).toBe("apps/local/echo");
    expect(form?.fields.map((f) => f.name)).toEqual(["word", "count", "mode"]);
    expect(form?.fields[0]?.type).toBe("str");
    expect(form?.fields[0]?.required).toBe(true);
    expect(form?.fields[1]?.required).toBe(false);
    expect(form?.fields[2]?.allowed).toEqual(["a", "b"]);
  });

  it("returns null for an absent / empty / malformed schema", () => {
    expect(appInputForm("h", null)).toBeNull();
    expect(appInputForm("h", undefined)).toBeNull();
    expect(appInputForm("h", {})).toBeNull();
    expect(appInputForm("h", { fields: [] })).toBeNull();
    expect(appInputForm("h", { fields: "nope" })).toBeNull();
    // a field with no name is dropped → no usable fields → null
    expect(appInputForm("h", { fields: [{ type: "str" }] })).toBeNull();
  });

  it("defaults an unknown type to str and accepts max_len snake_case", () => {
    const form = appInputForm("h", { fields: [{ name: "x", type: "weird", max_len: 12 }] });
    expect(form?.fields[0]?.type).toBe("str");
    expect(form?.fields[0]?.maxLen).toBe(12);
  });
});
