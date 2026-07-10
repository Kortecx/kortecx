import { describe, expect, it } from "vitest";
import { countSummary, formatMicroUsd, formatSeq, shortHex } from "../../src/lib/format";

describe("shortHex", () => {
  it("shortens a long hex id to head…tail", () => {
    const id = "ab".repeat(32); // 64 chars
    expect(shortHex(id)).toBe(`${id.slice(0, 8)}…${id.slice(-4)}`);
  });
  it("returns short input unchanged", () => {
    expect(shortHex("abcd")).toBe("abcd");
    expect(shortHex("")).toBe("");
  });
  it("honors custom head/tail", () => {
    const id = "0123456789abcdef".repeat(2);
    expect(shortHex(id, 4, 2)).toBe(`${id.slice(0, 4)}…${id.slice(-2)}`);
  });
});

describe("formatSeq", () => {
  it("null/undefined → em dash", () => {
    expect(formatSeq(null)).toBe("—");
    expect(formatSeq(undefined)).toBe("—");
  });
  it("number → #n", () => {
    expect(formatSeq(0)).toBe("#0");
    expect(formatSeq(42)).toBe("#42");
  });
});

describe("countSummary", () => {
  it("formats done/total noun", () => {
    expect(countSummary(3, 5, "committed")).toBe("3/5 committed");
  });
});

describe("formatMicroUsd", () => {
  it("formats a positive micro-USD amount as $x.xxxx", () => {
    expect(formatMicroUsd(1500)).toBe("$0.0015");
    expect(formatMicroUsd(1_000_000)).toBe("$1.0000");
  });
  it("returns the EMPTY string for zero / negative (GR15: never a fabricated $0.0000)", () => {
    expect(formatMicroUsd(0)).toBe("");
    expect(formatMicroUsd(-5)).toBe("");
  });
});
