/**
 * A session run history backed by localStorage (non-secret ids only), keyed per
 * endpoint so switching gateways never mixes histories. This is the FORWARD SEAM
 * for the additive `ListRuns` RPC (UI-2): `use-runs` reads this today; when ListRuns
 * lands, only the hook's source swaps — the `RunRecord` shape + the Runs view stay.
 *
 * Pure, fail-closed: a corrupt/unavailable store yields an empty list, never throws.
 */

export interface RunRecord {
  readonly instanceId: string;
  readonly terminalMoteId: string | null;
  readonly recipeFingerprint: string | null;
  /** The recipe handle invoked (when known). */
  readonly handle: string | null;
  /** Epoch ms the run was started from this console. */
  readonly startedAt: number;
}

/** Keep the session history bounded (newest-first). */
const MAX_RUNS = 50;

function keyFor(endpoint: string): string {
  return `kortecx.ui.runs:${endpoint}`;
}

export function loadRuns(endpoint: string): RunRecord[] {
  try {
    const raw = localStorage.getItem(keyFor(endpoint));
    if (raw === null) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.filter(isRunRecord).slice(0, MAX_RUNS);
  } catch {
    return [];
  }
}

/** Prepend a run (dedupe by instanceId, newest-first, bounded), return the list. */
export function recordRun(endpoint: string, run: RunRecord): RunRecord[] {
  const existing = loadRuns(endpoint).filter((r) => r.instanceId !== run.instanceId);
  const next = [run, ...existing].slice(0, MAX_RUNS);
  try {
    localStorage.setItem(keyFor(endpoint), JSON.stringify(next));
  } catch {
    /* best-effort */
  }
  return next;
}

export function clearRuns(endpoint: string): void {
  try {
    localStorage.removeItem(keyFor(endpoint));
  } catch {
    /* best-effort */
  }
}

function isRunRecord(v: unknown): v is RunRecord {
  if (v === null || typeof v !== "object") {
    return false;
  }
  const r = v as Record<string, unknown>;
  return typeof r.instanceId === "string" && typeof r.startedAt === "number";
}
