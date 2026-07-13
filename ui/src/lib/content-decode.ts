/**
 * Decode a committed artifact blob (the bytes `GetContent` returns) into a safe,
 * displayable shape. Pure + fail-closed: untrusted bytes are NEVER rendered as
 * HTML — JSON is pretty-printed as text, non-UTF-8 falls back to a bounded hex
 * preview. This is the only place run output crosses into the DOM, so the decode
 * decision lives here (single source) and the views just render `.text`.
 *
 * Multi-modal (the OSS Data Lab viewer): a magic-byte sniff classifies IMAGE /
 * VIDEO / AUDIO payloads, whose displayable shape is the raw `bytes` + a sniffed
 * `mediaType` (the viewer wraps them in a `blob:` object URL — never a remote
 * `src`, so there is no outbound-fetch / SSRF surface). MARKDOWN is opt-in via an
 * advisory hint (a `.md` filename or a `text/markdown` media type) — never a
 * fuzzy content heuristic, so a plain-text artifact never silently re-renders.
 */

import { decodeCriticVerdict } from "@kortecx/sdk/web";

/** A media MIME sniffed from magic bytes (or supplied as an advisory hint). */
type MediaKind = "image" | "video" | "audio";

export type DecodedKind =
  | "empty"
  | "json"
  | "text"
  | "markdown"
  | "html"
  | "binary"
  | "verdict"
  | MediaKind;

export interface DecodedContent {
  readonly kind: DecodedKind;
  /** The string the UI renders (pretty JSON, raw text, markdown source, or a hex
   *  preview). EMPTY for media kinds — those render from {@link bytes}. */
  readonly text: string;
  /** The parsed value when `kind === "json"` (objects/arrays only). */
  readonly json?: unknown;
  readonly byteLength: number;
  /** True when a binary/media preview was truncated (more bytes exist than shown). */
  readonly truncated: boolean;
  /** The raw bytes — present for media kinds (image/video/audio) so the viewer can
   *  build a `blob:` object URL. */
  readonly bytes?: Uint8Array;
  /** The sniffed (or advisory) MIME — present for media kinds. */
  readonly mediaType?: string;
}

/** Cap the hex preview so a large binary blob can't freeze the renderer. */
const MAX_HEX_BYTES = 2048;

function toHexPreview(bytes: Uint8Array): { text: string; truncated: boolean } {
  const n = Math.min(bytes.length, MAX_HEX_BYTES);
  const parts: string[] = [];
  for (let i = 0; i < n; i++) {
    // biome-ignore lint/style/noNonNullAssertion: i < n <= bytes.length
    parts.push(bytes[i]!.toString(16).padStart(2, "0"));
  }
  return { text: parts.join(" "), truncated: bytes.length > n };
}

/**
 * Sniff an image/video/audio MIME from a payload's magic bytes, or `null`. Covers
 * the common browser-renderable container formats (PNG/JPEG/GIF/WebP · MP4/WebM ·
 * MP3/WAV/OGG) — deterministic, allocation-free, never throws on a short buffer.
 */
export function sniffMediaMime(bytes: Uint8Array): string | null {
  const at = (i: number): number => bytes[i] ?? -1;
  // — Images —
  if (at(0) === 0x89 && at(1) === 0x50 && at(2) === 0x4e && at(3) === 0x47) {
    return "image/png";
  }
  if (at(0) === 0xff && at(1) === 0xd8 && at(2) === 0xff) {
    return "image/jpeg";
  }
  if (at(0) === 0x47 && at(1) === 0x49 && at(2) === 0x46 && at(3) === 0x38) {
    return "image/gif";
  }
  // RIFF container — WEBP (image) or WAVE (audio).
  if (at(0) === 0x52 && at(1) === 0x49 && at(2) === 0x46 && at(3) === 0x46) {
    if (at(8) === 0x57 && at(9) === 0x45 && at(10) === 0x42 && at(11) === 0x50) {
      return "image/webp";
    }
    if (at(8) === 0x57 && at(9) === 0x41 && at(10) === 0x56 && at(11) === 0x45) {
      return "audio/wav";
    }
  }
  // — Video —
  // ISO-BMFF / MP4: a `ftyp` box at offset 4.
  if (at(4) === 0x66 && at(5) === 0x74 && at(6) === 0x79 && at(7) === 0x70) {
    return "video/mp4";
  }
  // WebM / Matroska: the EBML header 1A 45 DF A3.
  if (at(0) === 0x1a && at(1) === 0x45 && at(2) === 0xdf && at(3) === 0xa3) {
    return "video/webm";
  }
  // — Audio —
  if (at(0) === 0x4f && at(1) === 0x67 && at(2) === 0x67 && at(3) === 0x53) {
    return "audio/ogg"; // "OggS"
  }
  if (at(0) === 0x49 && at(1) === 0x44 && at(2) === 0x33) {
    return "audio/mpeg"; // ID3-tagged MP3
  }
  // MP3 frame sync (no ID3 tag): FF Fx where x ∈ {B,3,2} (MPEG-1/2 layer III).
  if (at(0) === 0xff && (at(1) === 0xfb || at(1) === 0xf3 || at(1) === 0xf2)) {
    return "audio/mpeg";
  }
  return null;
}

/** Map a MIME to its media kind, or `null` (not a browser-renderable medium). */
export function mediaKindOf(mime: string): MediaKind | null {
  if (mime.startsWith("image/")) {
    return "image";
  }
  if (mime.startsWith("video/")) {
    return "video";
  }
  if (mime.startsWith("audio/")) {
    return "audio";
  }
  return null;
}

/** Decode hints (advisory only — never identity). */
export interface DecodeHints {
  /** An advisory MIME (e.g. an upload's recorded `media_type`). */
  readonly mediaType?: string;
  /** An advisory filename (e.g. an upload's name) — drives markdown selection. */
  readonly filename?: string;
}

const MARKDOWN_NAME = /\.(md|markdown)$/i;
const SVG_NAME = /\.svg$/i;
const HTML_NAME = /\.html?$/i;

function isMarkdownHint(hints: DecodeHints): boolean {
  if (hints.mediaType?.startsWith("text/markdown")) {
    return true;
  }
  return hints.filename !== undefined && MARKDOWN_NAME.test(hints.filename);
}

function isSvgHint(hints: DecodeHints): boolean {
  if (hints.mediaType?.startsWith("image/svg+xml")) {
    return true;
  }
  return hints.filename !== undefined && SVG_NAME.test(hints.filename);
}

function isHtmlHint(hints: DecodeHints): boolean {
  if (hints.mediaType?.startsWith("text/html")) {
    return true;
  }
  return hints.filename !== undefined && HTML_NAME.test(hints.filename);
}

export function decodeContent(bytes: Uint8Array, hints: DecodeHints = {}): DecodedContent {
  if (bytes.length === 0) {
    return { kind: "empty", text: "", byteLength: 0, truncated: false };
  }

  // T-AGENT2: a committed LLM-judge / critic verdict has a specific binary header
  // (2-byte schema version ‖ fixed-int variant) — recognize it BEFORE the text /
  // media classification, since its low control bytes decode as valid UTF-8. The
  // decoder is exact + conservative (version + variant must match), so a real text
  // / JSON payload is never mis-read as a verdict. Display-only (SN-8).
  const verdict = decodeCriticVerdict(bytes);
  if (verdict !== null) {
    return { kind: "verdict", text: verdict, byteLength: bytes.length, truncated: false };
  }

  // Media first: a magic-byte sniff, then an advisory image/video/audio hint — OR a
  // `.svg` filename (SVG is text, so it never magic-byte-sniffs). All resolve to the
  // raw bytes + MIME, rendered via a script-safe blob `<img>` (an <img>-loaded SVG
  // cannot execute embedded scripts or fetch external refs — no sanitizer needed).
  const advisory =
    (hints.mediaType && mediaKindOf(hints.mediaType) ? hints.mediaType : null) ??
    (isSvgHint(hints) ? "image/svg+xml" : null);
  const mime = sniffMediaMime(bytes) ?? advisory;
  if (mime) {
    const kind = mediaKindOf(mime);
    if (kind) {
      return {
        kind,
        text: "",
        bytes,
        mediaType: mime,
        byteLength: bytes.length,
        truncated: false,
      };
    }
  }

  let utf8: string | null = null;
  try {
    utf8 = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    utf8 = null;
  }

  if (utf8 === null) {
    const { text, truncated } = toHexPreview(bytes);
    return { kind: "binary", text, byteLength: bytes.length, truncated };
  }

  // Markdown + HTML are opt-in via an advisory hint (a `.md`/`.html` name or a
  // `text/markdown`/`text/html` media type) — NEVER guessed from content, so a plain-
  // text artifact never silently re-renders. Each keeps the decoded `text` (the source,
  // also the inline-edit path). SVG is handled above via the blob-`<img>` media path.
  if (isMarkdownHint(hints)) {
    return { kind: "markdown", text: utf8, byteLength: bytes.length, truncated: false };
  }
  if (isHtmlHint(hints)) {
    // The viewer renders HTML in a fully-sandboxed, CSP-locked iframe (no scripts, no
    // outbound fetch); `text` is the source (the edit path renders it too).
    return { kind: "html", text: utf8, byteLength: bytes.length, truncated: false };
  }

  // Only treat objects/arrays as JSON — a bare number/string is plain text.
  const trimmed = utf8.trim();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      const json: unknown = JSON.parse(trimmed);
      if (json !== null && typeof json === "object") {
        return {
          kind: "json",
          text: JSON.stringify(json, null, 2),
          json,
          byteLength: bytes.length,
          truncated: false,
        };
      }
    } catch {
      /* not valid JSON → fall through to plain text */
    }
  }

  return { kind: "text", text: utf8, byteLength: bytes.length, truncated: false };
}
