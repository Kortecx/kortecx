/**
 * Pure content-resolution helpers (Batch A): magic-byte image sniffing for the
 * chat attach path, a display kind classifier, and a ≤64-ref chunker over the
 * SDK's `getContentBatch` (the server refuses bigger batches — the chunker makes
 * the cap a non-event for callers). Network-free except `resolveRefs`, which
 * takes the SDK client explicitly (the hook layer owns React wiring).
 *
 * Security note: attachment previews render through `blob:` object URLs of the
 * user's OWN picked files — untrusted server bytes are never rendered as HTML
 * (the `content-decode` contract holds for text).
 */

import type { ContentItem, KxClient } from "@kortecx/sdk/web";
import { mediaKindOf, sniffMediaMime } from "./content-decode";

/** The gateway's batch ref cap (`MAX_BATCH_REFS`); bigger requests are refused. */
export const BATCH_REFS_MAX = 64;

/** The chat attach accept list — the image types the multimodal backend decodes. */
export const IMAGE_ACCEPT = "image/png,image/jpeg,image/webp,image/gif";

/** Sniff an image mime from magic bytes (PNG/JPEG/WebP/GIF), or `null`. */
export function sniffImageMime(bytes: Uint8Array): string | null {
  const at = (i: number): number => bytes[i] ?? -1;
  if (at(0) === 0x89 && at(1) === 0x50 && at(2) === 0x4e && at(3) === 0x47) {
    return "image/png";
  }
  if (at(0) === 0xff && at(1) === 0xd8 && at(2) === 0xff) {
    return "image/jpeg";
  }
  // RIFF....WEBP
  if (
    at(0) === 0x52 &&
    at(1) === 0x49 &&
    at(2) === 0x46 &&
    at(3) === 0x46 &&
    at(8) === 0x57 &&
    at(9) === 0x45 &&
    at(10) === 0x42 &&
    at(11) === 0x50
  ) {
    return "image/webp";
  }
  if (at(0) === 0x47 && at(1) === 0x49 && at(2) === 0x46 && at(3) === 0x38) {
    return "image/gif";
  }
  return null;
}

export type ResolvedKind = "image" | "video" | "audio" | "text" | "binary" | "missing";

/** One resolved batch item, classified for display. */
export interface ResolvedContent {
  readonly ref: string;
  readonly kind: ResolvedKind;
  readonly bytes: Uint8Array;
  readonly truncated: boolean;
  readonly fullSize: number;
  /** The sniffed mime when `kind` is a medium (image/video/audio). */
  readonly mediaType?: string;
}

/** Classify one batch item (the uniform empty item surfaces as `"missing"`). */
export function classifyItem(item: ContentItem): ResolvedContent {
  if (item.missing) {
    return {
      ref: item.contentRef,
      kind: "missing",
      bytes: item.payload,
      truncated: false,
      fullSize: 0,
    };
  }
  // Magic-byte media sniff (image/video/audio) — shared with content-decode so
  // the batch/preview path classifies media exactly like the single-blob viewer.
  const mime = sniffMediaMime(item.payload);
  const mediaKind = mime ? mediaKindOf(mime) : null;
  if (mediaKind !== null && mime !== null) {
    return {
      ref: item.contentRef,
      kind: mediaKind,
      bytes: item.payload,
      truncated: item.truncated,
      fullSize: Number(item.fullSize),
      mediaType: mime,
    };
  }
  // UTF-8 decodes ⇒ text; else binary (mirrors content-decode's fail-closed split).
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(item.payload);
    return {
      ref: item.contentRef,
      kind: "text",
      bytes: item.payload,
      truncated: item.truncated,
      fullSize: Number(item.fullSize),
    };
  } catch {
    return {
      ref: item.contentRef,
      kind: "binary",
      bytes: item.payload,
      truncated: item.truncated,
      fullSize: Number(item.fullSize),
    };
  }
}

/** Split `refs` into server-acceptable ≤64-ref chunks (order preserved). */
export function chunkRefs(refs: readonly string[]): string[][] {
  const out: string[][] = [];
  for (let i = 0; i < refs.length; i += BATCH_REFS_MAX) {
    out.push(refs.slice(i, i + BATCH_REFS_MAX));
  }
  return out;
}

/**
 * Resolve `refs` through `GetContentBatch`, chunked at the server cap, in
 * request order. `instanceId` scopes to a run; omitted reads the uploads scope.
 */
export async function resolveRefs(
  client: KxClient,
  refs: readonly string[],
  opts: { instanceId?: string; maxBytesPerItem?: bigint } = {},
): Promise<ResolvedContent[]> {
  const out: ResolvedContent[] = [];
  for (const chunk of chunkRefs(refs)) {
    const items = await client.getContentBatch(chunk, opts);
    out.push(...items.map(classifyItem));
  }
  return out;
}
