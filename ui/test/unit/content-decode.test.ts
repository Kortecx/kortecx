import { describe, expect, it } from "vitest";
import { decodeContent } from "../../src/lib/content-decode";

const utf8 = (s: string) => new TextEncoder().encode(s);

describe("decodeContent", () => {
  it("empty bytes → empty", () => {
    const d = decodeContent(new Uint8Array());
    expect(d.kind).toBe("empty");
    expect(d.byteLength).toBe(0);
    expect(d.text).toBe("");
  });

  it("plain UTF-8 text → text", () => {
    const d = decodeContent(utf8("hello, world"));
    expect(d.kind).toBe("text");
    expect(d.text).toBe("hello, world");
  });

  it("JSON object → pretty-printed json with parsed value", () => {
    const d = decodeContent(utf8('{"topic":"x","n":2}'));
    expect(d.kind).toBe("json");
    expect(d.json).toEqual({ topic: "x", n: 2 });
    expect(d.text).toBe(JSON.stringify({ topic: "x", n: 2 }, null, 2));
  });

  it("JSON array → json", () => {
    const d = decodeContent(utf8("[1,2,3]"));
    expect(d.kind).toBe("json");
    expect(d.json).toEqual([1, 2, 3]);
  });

  it("a bare number is text, not json", () => {
    const d = decodeContent(utf8("42"));
    expect(d.kind).toBe("text");
    expect(d.text).toBe("42");
  });

  it("invalid JSON that looks like an object falls back to text", () => {
    const d = decodeContent(utf8("{not json"));
    expect(d.kind).toBe("text");
  });

  it("non-UTF-8 bytes → binary hex preview", () => {
    const d = decodeContent(new Uint8Array([0xff, 0xfe, 0x00, 0x01]));
    expect(d.kind).toBe("binary");
    expect(d.text).toBe("ff fe 00 01");
    expect(d.truncated).toBe(false);
  });

  it("a large binary blob is truncated", () => {
    const big = new Uint8Array(5000).fill(0xff);
    const d = decodeContent(big);
    expect(d.kind).toBe("binary");
    expect(d.truncated).toBe(true);
    expect(d.byteLength).toBe(5000);
  });
});
