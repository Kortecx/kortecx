/**
 * Hex helpers for server-derived identifiers.
 *
 * The runtime computes every identifier (`MoteId`, `instance_id`, `content_ref`,
 * `terminal_mote_id`) — the SDK **never** derives one (SN-8). These helpers only
 * *encode* server bytes to lowercase hex for display and *decode* a user-supplied
 * hex string back to bytes (with strict length validation). There is deliberately
 * no "compute an id" function in this module or anywhere else in the SDK.
 */

import { KxUsage } from "./errors.js";

/** Length in bytes of a run instance id. */
export const INSTANCE_LEN = 16;
/** Length in bytes of a content ref / Mote id / signature id / digest. */
export const REF_LEN = 32;

/**
 * Render server bytes as lowercase hex (the SDK's display form for ids).
 *
 * @example
 * ```ts
 * encode(new Uint8Array([0xde, 0xad, 0xbe, 0xef])); // "deadbeef"
 * ```
 */
export function encode(data: Uint8Array): string {
  let s = "";
  for (const b of data) {
    s += b.toString(16).padStart(2, "0");
  }
  return s;
}

/** {@link encode}, but `null`/`undefined`-preserving (for optional refs). */
export function encodeOpt(data: Uint8Array | null | undefined): string | null {
  return data == null ? null : encode(data);
}

/**
 * Decode a hex string to bytes, throwing {@link KxUsage} on bad hex.
 *
 * @example
 * ```ts
 * decode("deadbeef");         // Uint8Array [0xde, 0xad, 0xbe, 0xef]
 * decode("nothex");           // throws KxUsage("invalid hex: nothex")
 * ```
 */
export function decode(s: string): Uint8Array {
  const t = s.trim();
  if (t.length % 2 !== 0 || (t.length > 0 && !/^[0-9a-fA-F]+$/.test(t))) {
    throw new KxUsage(`invalid hex: ${s}`);
  }
  const out = new Uint8Array(t.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = Number.parseInt(t.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/** Decode hex and require exactly `n` bytes (a length footgun guard). */
export function decodeFixed(s: string, n: number): Uint8Array {
  const b = decode(s);
  if (b.length !== n) {
    throw new KxUsage(`expected ${n} bytes (${n * 2} hex chars), got ${b.length} bytes`);
  }
  return b;
}

/**
 * Accept either a hex string or raw `n`-byte bytes; validate the length.
 *
 * This lets every SDK method take an id as the hex the rest of the SDK prints,
 * *or* as the raw bytes a previous response carried — both server-derived.
 *
 * @example
 * ```ts
 * asBytes("00112233445566778899aabbccddeeff", 16); // 16-byte Uint8Array
 * asBytes(someResponse.instanceId, 16);            // raw bytes pass through
 * ```
 */
export function asBytes(value: string | Uint8Array, n: number): Uint8Array {
  if (typeof value === "string") {
    return decodeFixed(value, n);
  }
  if (value instanceof Uint8Array) {
    if (value.length !== n) {
      throw new KxUsage(`expected ${n} bytes, got ${value.length}`);
    }
    return value;
  }
  throw new KxUsage(`expected a hex string or ${n} bytes, got ${typeof value}`);
}
