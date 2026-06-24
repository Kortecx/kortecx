/**
 * Pure language inference for the read-only code viewer. The offline Monaco
 * bundle ships JSON + plaintext + markdown (markdown is a tokenizer-only basic
 * language for the chat composer — see `setup.ts`); inference itself only ever
 * distinguishes JSON from plaintext, so this stays a tiny, total, unit-testable
 * function. An object/array that round-trips through `JSON.parse` is "json";
 * everything else is "plaintext". Mirrors `content-decode.ts`'s
 * "only objects/arrays are JSON" rule.
 */

export type MonacoLanguage = "json" | "plaintext" | "markdown";

export function inferLanguage(text: string): MonacoLanguage {
  const trimmed = text.trim();
  if (!(trimmed.startsWith("{") || trimmed.startsWith("["))) {
    return "plaintext";
  }
  try {
    const parsed: unknown = JSON.parse(trimmed);
    return parsed !== null && typeof parsed === "object" ? "json" : "plaintext";
  } catch {
    return "plaintext";
  }
}

/**
 * POC-5d: infer the editor language from a file PATH (the App project tree). The
 * offline Monaco bundle registers ONLY json + plaintext + markdown (see
 * `setup.ts`), so we map the three recognised extensions and fall back to
 * plaintext for everything else (no unregistered language is ever returned).
 * Total + pure — unit-tested directly.
 */
export function inferLanguageFromPath(path: string): MonacoLanguage {
  const lower = path.toLowerCase();
  if (lower.endsWith(".md") || lower.endsWith(".markdown")) {
    return "markdown";
  }
  if (lower.endsWith(".json")) {
    return "json";
  }
  return "plaintext";
}
