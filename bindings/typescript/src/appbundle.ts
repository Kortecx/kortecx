/**
 * `kortecx.appbundle/v1` — the portable App archive codec (browser-safe).
 *
 * An App bundle packages an App for portability: the canonical `AppEnvelope` bytes
 * plus the base64 closure of every content-store blob it references. The wire form
 * is a single canonical-JSON, all-strings document (sorted keys, compact) so this
 * SDK, the Rust `kx-appbundle` crate, and the Python SDK emit byte-identical
 * bundles — the cross-language contract is `tests/golden/apps/bundle_corpus.json`.
 *
 * This module owns the container format only. It validates structure — the schema
 * tag, 64-char lowercase-hex refs, and well-formed base64 — never cryptographic
 * identity: the runtime re-derives every blob ref and re-validates the envelope
 * server-side, so a bundle is a transport hint, never a trust boundary. No Node
 * `Buffer` / `fs` dependency, so it runs unchanged in the browser (the UI console).
 */

import { canonicalJson } from "./apps.js";

/** The bundle schema/version tag — readers fail closed on a mismatch. */
export const BUNDLE_SCHEMA = "kortecx.appbundle/v1";

/** Advisory import ceilings (H7) — bound the whole closure a bundle can carry
 *  (distinct from the server's per-blob 32 MiB PutContent cap). */
export const MAX_BUNDLE_REFS = 4096;
export const MAX_BUNDLE_CLOSURE_BYTES = 512 * 1024 * 1024; // 512 MiB

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** STANDARD base64 with `=` padding, single-line (never url-safe). Portable. */
function base64Encode(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i] ?? 0;
    const b1 = bytes[i + 1] ?? 0;
    const b2 = bytes[i + 2] ?? 0;
    out += B64.charAt(b0 >> 2);
    out += B64.charAt(((b0 & 0x03) << 4) | (b1 >> 4));
    out += i + 1 < bytes.length ? B64.charAt(((b1 & 0x0f) << 2) | (b2 >> 6)) : "=";
    out += i + 2 < bytes.length ? B64.charAt(b2 & 0x3f) : "=";
  }
  return out;
}

function base64Decode(s: string): Uint8Array {
  const clean = s.replace(/=+$/, "");
  const out = new Uint8Array((clean.length * 3) >> 2);
  let oi = 0;
  let buf = 0;
  let bits = 0;
  for (const ch of clean) {
    const v = B64.indexOf(ch);
    if (v < 0) throw new Error(`invalid base64: ${JSON.stringify(ch)}`);
    buf = (buf << 6) | v;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[oi++] = (buf >> bits) & 0xff;
    }
  }
  return out;
}

function checkHex(field: string, s: string): void {
  if (!/^[0-9a-f]{64}$/.test(s)) {
    throw new Error(`${field} must be 64-char lowercase hex, got ${JSON.stringify(s)}`);
  }
}

/** A decoded App bundle: the canonical envelope bytes + the raw content closure,
 *  named + tamper-checkable by the App's `appDigest` (verified by the runtime, not
 *  here). `sourceDigest` is an optional lineage hint (never authenticity). */
export class AppBundle {
  constructor(
    readonly appDigest: string,
    readonly envelope: Uint8Array,
    readonly blobs: Map<string, Uint8Array> = new Map(),
    readonly sourceDigest?: string,
  ) {}

  /** Serialize to the canonical `kortecx.appbundle/v1` wire string (sorted keys,
   *  compact, base64-STANDARD blobs) — byte-identical across Rust/Py/TS. */
  toJson(): string {
    const doc: Record<string, unknown> = {
      app_digest: this.appDigest,
      envelope: new TextDecoder().decode(this.envelope),
      schema: BUNDLE_SCHEMA,
    };
    if (this.blobs.size > 0) {
      const blobs: Record<string, string> = {};
      for (const [ref, body] of this.blobs) blobs[ref] = base64Encode(body);
      doc.blobs = blobs;
    }
    if (this.sourceDigest !== undefined) doc.source_digest = this.sourceDigest;
    return canonicalJson(doc);
  }

  /** Parse + structurally validate a wire string. Does NOT verify a blob hashes to
   *  its ref or that the envelope is valid — the runtime re-derives + re-validates
   *  those server-side. Throws on a schema mismatch, a bad hex ref, or bad base64. */
  static fromJson(wire: string): AppBundle {
    const doc = JSON.parse(wire) as Record<string, unknown>;
    if (doc.schema !== BUNDLE_SCHEMA) {
      throw new Error(
        `unsupported app bundle schema ${JSON.stringify(doc.schema)} (expected ${JSON.stringify(BUNDLE_SCHEMA)})`,
      );
    }
    const appDigest = String(doc.app_digest);
    checkHex("app_digest", appDigest);
    const sourceDigest = doc.source_digest === undefined ? undefined : String(doc.source_digest);
    if (sourceDigest !== undefined) checkHex("source_digest", sourceDigest);
    const blobs = new Map<string, Uint8Array>();
    const rawBlobs = (doc.blobs ?? {}) as Record<string, string>;
    for (const [ref, b64] of Object.entries(rawBlobs)) {
      checkHex("blob ref", ref);
      blobs.set(ref, base64Decode(b64));
    }
    return new AppBundle(
      appDigest,
      new TextEncoder().encode(String(doc.envelope)),
      blobs,
      sourceDigest,
    );
  }

  /** Total raw byte size of the content closure (for an import ceiling). */
  totalBlobBytes(): number {
    let n = 0;
    for (const b of this.blobs.values()) n += b.length;
    return n;
  }

  /** Number of blobs in the content closure. */
  blobCount(): number {
    return this.blobs.size;
  }
}
