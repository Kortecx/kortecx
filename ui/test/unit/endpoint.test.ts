import { describe, expect, it } from "vitest";
import { isNonloopbackPlaintext, validateEndpoint } from "../../src/lib/endpoint";

describe("validateEndpoint", () => {
  it("rejects empty", () => {
    expect(validateEndpoint("")).toMatch(/required/);
    expect(validateEndpoint("   ")).toMatch(/required/);
  });
  it("rejects a missing scheme", () => {
    expect(validateEndpoint("127.0.0.1:50151")).toMatch(/http/);
  });
  it("accepts a valid http/https endpoint", () => {
    expect(validateEndpoint("http://127.0.0.1:50151")).toBeNull();
    expect(validateEndpoint("https://gw.example.com")).toBeNull();
  });
});

describe("isNonloopbackPlaintext (SDK re-export)", () => {
  it("loopback http is not flagged", () => {
    expect(isNonloopbackPlaintext("http://127.0.0.1:50151")).toBe(false);
    expect(isNonloopbackPlaintext("http://localhost:50151")).toBe(false);
  });
  it("remote http IS flagged", () => {
    expect(isNonloopbackPlaintext("http://example.com:50151")).toBe(true);
    expect(isNonloopbackPlaintext("http://10.0.0.5")).toBe(true);
  });
  it("https is never flagged", () => {
    expect(isNonloopbackPlaintext("https://example.com")).toBe(false);
  });
});
