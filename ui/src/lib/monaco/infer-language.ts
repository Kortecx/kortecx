/**
 * Pure language inference for the read-only code viewer. We only ever distinguish
 * JSON from plaintext (the two languages the offline Monaco bundle ships — see
 * `setup.ts`), so this stays a tiny, total, unit-testable function. An object/array
 * that round-trips through `JSON.parse` is "json"; everything else is "plaintext".
 * Mirrors `content-decode.ts`'s "only objects/arrays are JSON" rule.
 */

export type MonacoLanguage = "json" | "plaintext";

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
