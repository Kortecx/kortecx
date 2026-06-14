/**
 * The "clean name" rule for the Workflows + Blueprints cards (PR-4.1b): turn a
 * wire handle like `"kx/recipes/echo"` into a readable display headline
 * (`"Echo"`) while the raw handle stays available as a secondary mono chip.
 * PURE + total — no React, no SDK. DISPLAY ONLY: identity never derives from
 * this (SN-8); the handle on the wire is unchanged.
 */

/** The bare leaf of a slash-separated handle (`"kx/recipes/echo"` → `"echo"`). */
export function handleLeaf(handle: string): string {
  const trimmed = handle.trim().replace(/\/+$/, "");
  const slash = trimmed.lastIndexOf("/");
  return slash === -1 ? trimmed : trimmed.slice(slash + 1);
}

/**
 * A humanized display name: the leaf with `-`/`_` separators turned to spaces
 * and each word Title-Cased (`"kx/recipes/agent_loop"` → `"Agent Loop"`). An
 * empty/whitespace handle returns the trimmed input unchanged so the caller can
 * fall back to a hex id.
 */
export function humanizeHandle(handle: string): string {
  const leaf = handleLeaf(handle);
  if (leaf === "") {
    return handle.trim();
  }
  return leaf
    .replace(/[-_]+/g, " ")
    .split(/\s+/)
    .filter((w) => w.length > 0)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}
