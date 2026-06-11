import { describe, expect, it } from "vitest";
import { inferLanguage } from "../../src/lib/monaco/infer-language";

describe("inferLanguage", () => {
  it("recognizes JSON objects and arrays", () => {
    expect(inferLanguage('{"a":1}')).toBe("json");
    expect(inferLanguage("  [1, 2, 3]  ")).toBe("json");
    expect(inferLanguage('{\n  "topic": "hi"\n}')).toBe("json");
  });

  it("treats bare scalars + non-JSON as plaintext", () => {
    expect(inferLanguage("42")).toBe("plaintext");
    expect(inferLanguage('"just a string"')).toBe("plaintext");
    expect(inferLanguage("hello world")).toBe("plaintext");
    expect(inferLanguage("")).toBe("plaintext");
  });

  it("treats object-looking-but-invalid JSON as plaintext", () => {
    expect(inferLanguage("{not valid")).toBe("plaintext");
    expect(inferLanguage("[1, 2,")).toBe("plaintext");
  });
});
