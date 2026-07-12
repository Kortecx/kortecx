/**
 * PR-A: the pure decoder for a Mote's `config_subset["kx.context.items"]` value —
 * the grounded chunk refs the chat-rag bind layer folds into the answer Mote. Mirrors
 * the Rust wire format `u32-le(name.len()) ‖ name ‖ ref[32]` and its fail-soft
 * truncation behaviour (`crates/kx-mote/src/context_items.rs`).
 */

import { describe, expect, it } from "vitest";
import { decodeContextItems } from "../../src/lib/context-items";

/** Encode one item exactly as the Rust `encode_context_items` does (a 32-byte ref
 *  filled with `refByte`, so the expected hex is `refByte` repeated 32×). */
function encodeItem(name: string, refByte: number): Uint8Array {
  const nameBytes = new TextEncoder().encode(name);
  const buf = new Uint8Array(4 + nameBytes.length + 32);
  new DataView(buf.buffer).setUint32(0, nameBytes.length, true);
  buf.set(nameBytes, 4);
  buf.fill(refByte, 4 + nameBytes.length);
  return buf;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let i = 0;
  for (const p of parts) {
    out.set(p, i);
    i += p.length;
  }
  return out;
}

const hex = (byte: number) => byte.toString(16).padStart(2, "0").repeat(32);

describe("decodeContextItems (PR-A grounding refs)", () => {
  it("decodes an empty buffer to no items", () => {
    expect(decodeContextItems(new Uint8Array())).toEqual([]);
  });

  it("round-trips a single item's label + 64-hex ref", () => {
    const dec = decodeContextItems(encodeItem("spec.md", 0x11));
    expect(dec).toEqual([{ label: "spec.md", ref: hex(0x11) }]);
  });

  it("decodes multiple items in encoded order", () => {
    const bytes = concat(encodeItem("a", 0x11), encodeItem("beta", 0x22), encodeItem("c", 0x33));
    expect(decodeContextItems(bytes)).toEqual([
      { label: "a", ref: hex(0x11) },
      { label: "beta", ref: hex(0x22) },
      { label: "c", ref: hex(0x33) },
    ]);
  });

  it("fail-soft: a truncated trailing item is dropped, prior items survive", () => {
    const good = encodeItem("first", 0x44);
    const truncated = encodeItem("second", 0x55).subarray(0, 6); // len header + partial
    expect(decodeContextItems(concat(good, truncated))).toEqual([
      { label: "first", ref: hex(0x44) },
    ]);
  });

  it("fail-soft: a length header that overruns the buffer stops cleanly (no throw)", () => {
    const buf = new Uint8Array(8); // a u32 len of 0 then 4 stray bytes < 32 → no item
    new DataView(buf.buffer).setUint32(0, 0xffff, true); // absurd len ⇒ overrun ⇒ stop
    expect(decodeContextItems(buf)).toEqual([]);
  });

  it("decodes correctly from a subarray view (non-zero byteOffset)", () => {
    const padded = concat(new Uint8Array([0, 0, 0]), encodeItem("x", 0x66));
    expect(decodeContextItems(padded.subarray(3))).toEqual([{ label: "x", ref: hex(0x66) }]);
  });
});
