import { describe, expect, it } from "vitest";
import { inferLanguage, inferLanguageFromPath } from "../../src/lib/monaco/infer-language";

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

describe("inferLanguageFromPath (POC-5d App project tree)", () => {
  it("maps the three registered Monaco languages by extension", () => {
    expect(inferLanguageFromPath("README.md")).toBe("markdown");
    expect(inferLanguageFromPath("prompts/system.md")).toBe("markdown");
    expect(inferLanguageFromPath("app.json")).toBe("json");
    expect(inferLanguageFromPath("README.MD")).toBe("markdown");
    expect(inferLanguageFromPath("APP.JSON")).toBe("json");
  });

  it("falls back to plaintext for any unregistered extension", () => {
    expect(inferLanguageFromPath("src/main.rs")).toBe("plaintext");
    expect(inferLanguageFromPath("notes.txt")).toBe("plaintext");
    expect(inferLanguageFromPath("Dockerfile")).toBe("plaintext");
    expect(inferLanguageFromPath("")).toBe("plaintext");
  });
});
