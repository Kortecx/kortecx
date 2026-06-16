import { describe, expect, it } from "vitest";
import {
  BATCH_REFS_MAX,
  chunkRefs,
  classifyItem,
  sniffImageMime,
} from "../../src/lib/content-resolver";

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const JPEG = new Uint8Array([0xff, 0xd8, 0xff, 0xe0]);
const WEBP = new Uint8Array([
  0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50,
]);
const GIF = new Uint8Array([0x47, 0x49, 0x46, 0x38, 0x39, 0x61]);

describe("sniffImageMime", () => {
  it("recognizes the four supported image magics", () => {
    expect(sniffImageMime(PNG)).toBe("image/png");
    expect(sniffImageMime(JPEG)).toBe("image/jpeg");
    expect(sniffImageMime(WEBP)).toBe("image/webp");
    expect(sniffImageMime(GIF)).toBe("image/gif");
  });

  it("returns null for text, empty, and short buffers", () => {
    expect(sniffImageMime(new TextEncoder().encode("hello"))).toBeNull();
    expect(sniffImageMime(new Uint8Array())).toBeNull();
    expect(sniffImageMime(new Uint8Array([0x89]))).toBeNull();
  });
});

interface FakeItem {
  contentRef: string;
  payload: Uint8Array;
  truncated: boolean;
  fullSize: bigint;
  missing: boolean;
  text: string;
}

function item(payload: Uint8Array, opts: Partial<FakeItem> = {}): FakeItem {
  return {
    contentRef: "ab".repeat(32),
    payload,
    truncated: false,
    fullSize: BigInt(payload.length),
    missing: payload.length === 0,
    text: "",
    ...opts,
  };
}

describe("classifyItem", () => {
  it("classifies an image payload with its sniffed mime", () => {
    const r = classifyItem(item(PNG));
    expect(r.kind).toBe("image");
    expect(r.mediaType).toBe("image/png");
  });

  it("classifies UTF-8 as text and non-UTF-8 as binary", () => {
    expect(classifyItem(item(new TextEncoder().encode("plain"))).kind).toBe("text");
    expect(classifyItem(item(new Uint8Array([0xff, 0xfe, 0x00, 0x01]))).kind).toBe("binary");
  });

  it("classifies video + audio payloads via the shared media sniff", () => {
    const mp4 = new Uint8Array(16);
    mp4.set([0x66, 0x74, 0x79, 0x70], 4); // 'ftyp' at offset 4
    expect(classifyItem(item(mp4)).kind).toBe("video");
    const ogg = new Uint8Array([0x4f, 0x67, 0x67, 0x53, 0, 0, 0, 0]);
    const r = classifyItem(item(ogg));
    expect(r.kind).toBe("audio");
    expect(r.mediaType).toBe("audio/ogg");
  });

  it("surfaces the UNIFORM empty item as missing (no existence oracle)", () => {
    const r = classifyItem(item(new Uint8Array(), { fullSize: 0n, missing: true }));
    expect(r.kind).toBe("missing");
    expect(r.fullSize).toBe(0);
  });

  it("keeps honest truncation metadata", () => {
    const r = classifyItem(
      item(new TextEncoder().encode("par"), {
        truncated: true,
        fullSize: 999n,
        missing: false,
      }),
    );
    expect(r.truncated).toBe(true);
    expect(r.fullSize).toBe(999);
  });
});

describe("chunkRefs", () => {
  it("splits at the server's 64-ref cap, order preserved", () => {
    const refs = Array.from({ length: 130 }, (_, i) => `r${i}`);
    const chunks = chunkRefs(refs);
    expect(chunks.map((c) => c.length)).toEqual([BATCH_REFS_MAX, BATCH_REFS_MAX, 2]);
    expect(chunks[0]?.[0]).toBe("r0");
    expect(chunks[2]?.[1]).toBe("r129");
  });

  it("handles empty and exactly-at-cap inputs", () => {
    expect(chunkRefs([])).toEqual([]);
    expect(chunkRefs(Array.from({ length: 64 }, (_, i) => `r${i}`))).toHaveLength(1);
  });
});
