/**
 * POC-5a / POC-6: the pure scaffold-progress derivation. An App's "New App"
 * agentic scaffold writes a DYNAMIC, use-case-specific project tree into the App's
 * CoW branch (a scheduled app plans its file set with the model; a hosted app uses
 * its framework template). This module maps the server's LIVE {@link ScaffoldStatus}
 * (the real `filesDone` / `filesPending` lists + `phase` + the live-writing ids)
 * onto per-path row states — the file set is the SERVER's truth, never a fixed list.
 *
 * HONEST (GR15 / D142.3): the row state is driven ONLY by the server-reported
 * facts — never a timer, never a fabricated "done". A file is `done` iff the
 * server lists it in `filesDone`; `writing` iff it is the server's `writingPath`
 * (POC-6) — or, for an older server, the first not-done pending path while
 * actively writing; otherwise `pending`. A `failed` phase leaves the partial
 * files exactly as the server reported them (no fake completion).
 *
 * Pure + total (no React, no I/O) so the mapping is unit-tested directly.
 */

import type { ScaffoldStatus } from "@kortecx/sdk/web";

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
  /** Convenience: the server reported the scaffold done. */
  readonly complete: boolean;
  /** Convenience: the server reported a failure (show `detail`, keep partials). */
  readonly failed: boolean;
  /** POC-6: the path being authored right now (the live-stream target), if any. */
  readonly writingPath?: string;
  /** POC-6: the run instance streaming the writing file's tokens (hex), if any. */
  readonly writingInstanceId?: string;
  /** POC-6: the write mote whose decode streams the writing file (hex), if any. */
  readonly writingMoteId?: string;
}

/**
 * Map the DYNAMIC project file set onto per-path row states from the REAL server
 * status. The planned set is `filesDone ∪ filesPending` (the server's truth),
 * deduped, done-first.
 *
 * - `done`    — the server reports the path in `filesDone`.
 * - `writing` — the path equals the server's `writingPath` (POC-6); else, for an
 *               older server, the FIRST not-yet-done path while actively writing.
 * - `pending` — everything else (incl. all rows while `planning`, and every
 *               not-done row once `failed`/`done` — no fabricated progress).
 */
export function deriveScaffoldStatus(status: ScaffoldStatus): DerivedScaffold {
  const done = new Set(status.filesDone);

  // The planned set — the server's own file list (done-first), deduped defensively.
  const seen = new Set<string>();
  const paths: string[] = [];
  for (const p of [...status.filesDone, ...status.filesPending]) {
    if (!seen.has(p)) {
      seen.add(p);
      paths.push(p);
    }
  }

  const writing = status.phase === "writing";
  // Prefer the SERVER's writingPath (POC-6); else fall back to the first not-done
  // pending path while actively writing (an older server). Honest — one spinner.
  const writingPath =
    status.writingPath || (writing ? status.filesPending.find((p) => !done.has(p)) : undefined);

  const rows: ScaffoldRow[] = paths.map((path) => {
    if (done.has(path)) {
      return { path, state: "done" };
    }
    if (path === writingPath) {
      return { path, state: "writing" };
    }
    return { path, state: "pending" };
  });

  const complete = status.phase === "done";
  const failed = status.phase === "failed";
  const active = status.phase === "planning" || status.phase === "writing";

  return {
    rows,
    phase: status.phase,
    active,
    complete,
    failed,
    writingPath: writingPath || undefined,
    // The live stream needs BOTH ids; carry them only while a file is being written.
    writingInstanceId: writingPath ? status.writingInstanceId : undefined,
    writingMoteId: writingPath ? status.writingMoteId : undefined,
  };
}
