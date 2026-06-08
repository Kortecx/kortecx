/**
 * A session run history backed by localStorage (non-secret ids only), keyed per
 * endpoint so switching gateways never mixes histories. UI-2 MERGES this with the
 * durable `ListRuns` enumeration (see {@link mergeServerRuns}): the local records
 * carry the richer per-invocation handle + terminal Mote; the server adds any
 * durable instance the local history doesn't already cover (e.g. after a reload).
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

/** The fields `mergeServerRuns` needs from a `ListRuns` `RunSummary` (SDK-free so
 *  this module stays a pure lib with no SDK import). */
export interface ServerRun {
  readonly instanceId: string;
  readonly recipeFingerprint: string;
  readonly registeredUnixMs: number;
}

/**
 * Merge the richer local records with the durable server runs: keep every local
 * record (per-invocation handle + terminal), then append any server instance the
 * local history does not already cover (as a bare durable card), newest-first.
 */
export function mergeServerRuns(local: RunRecord[], server: ServerRun[]): RunRecord[] {
  const seen = new Set(local.map((r) => r.instanceId));
  const serverOnly: RunRecord[] = server
    .filter((s) => !seen.has(s.instanceId))
    .map((s) => ({
      instanceId: s.instanceId,
      terminalMoteId: null,
      recipeFingerprint: s.recipeFingerprint,
      handle: null,
      startedAt: s.registeredUnixMs,
    }));
  return [...local, ...serverOnly].sort((a, b) => b.startedAt - a.startedAt);
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
