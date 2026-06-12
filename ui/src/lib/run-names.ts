/**
 * Client-local run display names (PR-2.1), keyed per endpoint — the
 * `recent-runs.ts` pattern. PRESENTATION state only: the wire has no run-name
 * field (a durable names sidecar is a later batch), so a rename lives in THIS
 * browser and never leaves it. Fail-closed: a corrupt/unavailable store reads
 * as "no names", never throws.
 */

const MAX_NAMES = 200;

/** Fired on `window` whenever the persisted names change (same-tab freshness —
 *  the `RUNS_CHANGED_EVENT` rationale). */
export const RUN_NAMES_CHANGED_EVENT = "kortecx:run-names-changed";

function notifyChanged(): void {
  try {
    window.dispatchEvent(new Event(RUN_NAMES_CHANGED_EVENT));
  } catch {
    /* non-browser env */
  }
}

function keyFor(endpoint: string): string {
  return `kortecx.ui.run-names:${endpoint}`;
}

export function loadRunNames(endpoint: string): Record<string, string> {
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

/** Set (or clear, with an empty string) the local name for `instanceId`. */
export function setRunName(endpoint: string, instanceId: string, name: string): void {
  const names = loadRunNames(endpoint);
  const trimmed = name.trim();
  if (trimmed === "") {
    delete names[instanceId];
  } else {
    names[instanceId] = trimmed.slice(0, 120);
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
