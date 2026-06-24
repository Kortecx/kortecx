/**
 * POC-5a: the pure scaffold-progress derivation. An App's "New App" agentic
 * scaffold writes a FIXED skeleton project tree into the App's CoW branch; this
 * module maps the server's LIVE {@link ScaffoldStatus} (the real `filesDone` /
 * `filesPending` lists + `phase`) onto a stable per-path row state.
 *
 * HONEST (GR15 / D142.3): the row state is driven ONLY by the server-reported
 * facts — never a timer, never a fabricated "done". A file is `done` iff the
 * server says so; `writing` iff the scaffold is actively writing AND the file is
 * the next pending one; otherwise `pending`. A `failed` phase leaves the partial
 * files exactly as the server reported them (no fake completion).
 *
 * Pure + total (no React, no I/O) so the mapping is unit-tested directly.
 */

import type { ScaffoldStatus } from "@kortecx/sdk/web";

/** The fixed skeleton tree the server scaffolds (mirrors the server's set). The
 *  UI renders ALL of these so progress is honest even before the first file
 *  lands — a missing path reads as `pending`, never absent. */
export const SKELETON_PATHS: readonly string[] = [
  "README.md",
  "app.json",
  "prompts/system.md",
  "rules/guardrails.md",
  "skills/main.md",
] as const;

export type ScaffoldRowState = "done" | "writing" | "pending";

export interface ScaffoldRow {
  readonly path: string;
  readonly state: ScaffoldRowState;
}

export interface DerivedScaffold {
  readonly rows: readonly ScaffoldRow[];
  /** The overall phase, passed through from the server (display + polling gate). */
  readonly phase: ScaffoldStatus["phase"];
  /** True while the scaffold is actively running (poll while this holds). */
  readonly active: boolean;
  /** Convenience: every skeleton path is reported done. */
  readonly complete: boolean;
  /** Convenience: the server reported a failure (show `detail`, keep partials). */
  readonly failed: boolean;
}

/**
 * Map the fixed skeleton onto per-path row states from the REAL server status.
 *
 * - `done`    — the server reports the path in `filesDone`.
 * - `writing` — phase is `writing` AND the path is the FIRST not-yet-done
 *               skeleton path (the one being written next); never more than one.
 * - `pending` — everything else (incl. all rows while `planning`, and every
 *               not-done row once `failed`/`done` — no fabricated progress).
 */
export function deriveScaffoldStatus(
  skeletonPaths: readonly string[],
  status: ScaffoldStatus,
): DerivedScaffold {
  const done = new Set(status.filesDone);
  const writing = status.phase === "writing";
  // The single in-flight row: the first skeleton path the server has NOT marked
  // done while it is actively writing. Honest (one spinner, never many).
  const writingPath = writing ? skeletonPaths.find((p) => !done.has(p)) : undefined;

  const rows: ScaffoldRow[] = skeletonPaths.map((path) => {
    if (done.has(path)) {
      return { path, state: "done" };
    }
    if (path === writingPath) {
      return { path, state: "writing" };
    }
    return { path, state: "pending" };
  });

  const complete = skeletonPaths.length > 0 && skeletonPaths.every((p) => done.has(p));
  const failed = status.phase === "failed";
  const active = status.phase === "planning" || status.phase === "writing";

  return { rows, phase: status.phase, active, complete, failed };
}
