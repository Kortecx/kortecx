import { describe, expect, it } from "vitest";
import { decodeContent, mediaKindOf, sniffMediaMime } from "../../src/lib/content-decode";

const utf8 = (s: string) => new TextEncoder().encode(s);

/** A buffer that starts with `magic` (padded so offset sniffs have room). */
const withMagic = (magic: number[], len = 32) => {
  const b = new Uint8Array(len);
  b.set(magic, 0);
  return b;
};

// T-AGENT2: canonical `CriticVerdict::encode` bytes (2-byte LE schema version ‖
// fixed-int bincode) — must match the Rust/SDK byte layout.
const VERDICT_VALID = new Uint8Array([1, 0, 0, 0, 0, 0]);
const VERDICT_INVALID_JUDGE = new Uint8Array([1, 0, 1, 0, 0, 0, 5, 0, 0, 0, 0, 0]);

describe("decodeContent", () => {
  it("empty bytes → empty", () => {
    const d = decodeContent(new Uint8Array());
    expect(d.kind).toBe("empty");
    expect(d.byteLength).toBe(0);
    expect(d.text).toBe("");
  });

  it("a committed VALID judge verdict → verdict kind", () => {
    const d = decodeContent(VERDICT_VALID);
    expect(d.kind).toBe("verdict");
    expect(d.text).toBe("valid");
  });

  it("a committed INVALID judge verdict → verdict kind with the reason", () => {
    const d = decodeContent(VERDICT_INVALID_JUDGE);
    expect(d.kind).toBe("verdict");
    expect(d.text).toBe("invalid: judge: answer did not satisfy the rubric");
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

  it("PNG magic → image, carrying the raw bytes + sniffed mime", () => {
    const png = withMagic([0x89, 0x50, 0x4e, 0x47]);
    const d = decodeContent(png);
    expect(d.kind).toBe("image");
    expect(d.mediaType).toBe("image/png");
    expect(d.bytes).toBe(png);
    expect(d.text).toBe("");
  });

  it("MP4 ftyp box → video; OggS → audio; RIFF WAVE → audio", () => {
    // 'ftyp' at offset 4
    expect(decodeContent(withMagic([0, 0, 0, 0, 0x66, 0x74, 0x79, 0x70])).kind).toBe("video");
    expect(decodeContent(withMagic([0x4f, 0x67, 0x67, 0x53])).kind).toBe("audio");
    // RIFF....WAVE
    expect(
      decodeContent(withMagic([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x41, 0x56, 0x45])).kind,
    ).toBe("audio");
  });

  it("an advisory image media type promotes non-sniffable bytes (e.g. SVG text) to image", () => {
    const d = decodeContent(utf8("<svg/>"), { mediaType: "image/svg+xml" });
    expect(d.kind).toBe("image");
    expect(d.mediaType).toBe("image/svg+xml");
  });

  it("markdown is opt-in via a hint, never guessed from content", () => {
    expect(decodeContent(utf8("# Title\n\ntext")).kind).toBe("text");
    expect(decodeContent(utf8("# Title"), { filename: "readme.md" }).kind).toBe("markdown");
    expect(decodeContent(utf8("hi"), { mediaType: "text/markdown" }).kind).toBe("markdown");
  });

  it("a `.svg` filename (no media type) promotes SVG text to the script-safe image path", () => {
    const d = decodeContent(utf8("<svg xmlns='http://www.w3.org/2000/svg'/>"), {
      filename: "diagram.svg",
    });
    expect(d.kind).toBe("image");
    expect(d.mediaType).toBe("image/svg+xml");
    expect(d.bytes).toBeDefined();
  });

  it("HTML is opt-in via a hint (its own sandboxed kind), never guessed from content", () => {
    // No hint → plain text (fail-closed; never silently rendered as live HTML).
    expect(decodeContent(utf8("<h1>hi</h1>")).kind).toBe("text");
    expect(decodeContent(utf8("<h1>hi</h1>"), { filename: "report.html" }).kind).toBe("html");
    expect(decodeContent(utf8("<x>"), { filename: "a.htm" }).kind).toBe("html");
    expect(decodeContent(utf8("<b>hi</b>"), { mediaType: "text/html" }).kind).toBe("html");
    // The source is kept for the inline-edit path.
    expect(decodeContent(utf8("<h1>hi</h1>"), { filename: "r.html" }).text).toBe("<h1>hi</h1>");
  });
});

describe("sniffMediaMime / mediaKindOf", () => {
  it("recognizes the common browser-renderable formats", () => {
    expect(sniffMediaMime(withMagic([0x89, 0x50, 0x4e, 0x47]))).toBe("image/png");
    expect(sniffMediaMime(withMagic([0xff, 0xd8, 0xff]))).toBe("image/jpeg");
    expect(sniffMediaMime(withMagic([0x47, 0x49, 0x46, 0x38]))).toBe("image/gif");
    expect(
      sniffMediaMime(withMagic([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50])),
    ).toBe("image/webp");
    expect(sniffMediaMime(withMagic([0x1a, 0x45, 0xdf, 0xa3]))).toBe("video/webm");
    expect(sniffMediaMime(withMagic([0x49, 0x44, 0x33]))).toBe("audio/mpeg");
    expect(sniffMediaMime(withMagic([0xff, 0xfb]))).toBe("audio/mpeg");
  });

  it("returns null for text + short buffers (fail-closed, never throws)", () => {
    expect(sniffMediaMime(utf8("hello"))).toBeNull();
    expect(sniffMediaMime(new Uint8Array())).toBeNull();
    expect(sniffMediaMime(new Uint8Array([0x89]))).toBeNull();
  });

  it("mediaKindOf maps a MIME prefix to its kind", () => {
    expect(mediaKindOf("image/png")).toBe("image");
    expect(mediaKindOf("video/mp4")).toBe("video");
    expect(mediaKindOf("audio/ogg")).toBe("audio");
    expect(mediaKindOf("text/plain")).toBeNull();
  });
});
