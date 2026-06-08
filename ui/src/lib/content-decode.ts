/**
 * Decode a committed artifact blob (the bytes `GetContent` returns) into a safe,
 * displayable shape. Pure + fail-closed: untrusted bytes are NEVER rendered as
 * HTML — JSON is pretty-printed as text, non-UTF-8 falls back to a bounded hex
 * preview. This is the only place run output crosses into the DOM, so the decode
 * decision lives here (single source) and the views just render `.text`.
 */

export type DecodedKind = "empty" | "json" | "text" | "binary";

export interface DecodedContent {
  readonly kind: DecodedKind;
  /** The string the UI renders (pretty JSON, raw text, or a hex preview). */
  readonly text: string;
  /** The parsed value when `kind === "json"` (objects/arrays only). */
  readonly json?: unknown;
  readonly byteLength: number;
  /** True when a binary preview was truncated (more bytes exist than shown). */
  readonly truncated: boolean;
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

export function decodeContent(bytes: Uint8Array): DecodedContent {
  if (bytes.length === 0) {
    return { kind: "empty", text: "", byteLength: 0, truncated: false };
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
