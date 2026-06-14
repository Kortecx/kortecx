/**
 * Client-local Blueprint display names (PR-4.1b), keyed per endpoint by the
 * stable wire HANDLE — the `run-names.ts` pattern. PRESENTATION state only: the
 * wire has no blueprint-name field, so a rename lives in THIS browser and never
 * leaves it. Fail-closed: a corrupt/unavailable store reads as "no names",
 * never throws.
 */

const MAX_NAMES = 200;

/** Fired on `window` whenever the persisted names change (same-tab freshness —
 *  the `RUN_NAMES_CHANGED_EVENT` rationale). */
export const BLUEPRINT_NAMES_CHANGED_EVENT = "kortecx:blueprint-names-changed";

function notifyChanged(): void {
  try {
    window.dispatchEvent(new Event(BLUEPRINT_NAMES_CHANGED_EVENT));
  } catch {
    /* non-browser env */
  }
}

function keyFor(endpoint: string): string {
  return `kortecx.ui.blueprint-names:${endpoint}`;
}

export function loadBlueprintNames(endpoint: string): Record<string, string> {
  try {
    const raw = localStorage.getItem(keyFor(endpoint));
    if (raw === null) {
      return {};
    }
    const parsed: unknown = JSON.parse(raw);
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof v === "string" && v.trim() !== "") {
        out[k] = v;
      }
    }
    return out;
  } catch {
    return {};
  }
}

/** Set (or clear, with an empty string) the local name for a blueprint `handle`. */
export function setBlueprintName(endpoint: string, handle: string, name: string): void {
  const names = loadBlueprintNames(endpoint);
  const trimmed = name.trim();
  if (trimmed === "") {
    delete names[handle];
  } else {
    names[handle] = trimmed.slice(0, 120);
  }
  // Bounded: drop arbitrary overflow entries (insertion order is good enough
  // for a presentation map).
  const entries = Object.entries(names).slice(-MAX_NAMES);
  try {
    localStorage.setItem(keyFor(endpoint), JSON.stringify(Object.fromEntries(entries)));
  } catch {
    /* best-effort */
  }
  notifyChanged();
}
