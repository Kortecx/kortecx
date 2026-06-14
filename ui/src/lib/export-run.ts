/**
 * Serialize a run to a stable, self-describing JSON document (the Workflows
 * card "Export" affordance). A pure transform: the lightweight envelope is the
 * client-known {@link RunRecord}; the OPTIONAL {@link RunBundle} (the committed
 * DAG + each Mote's resolved output text) is assembled by the impure
 * `use-run-export` hook from a fetched `GetProjection`/`GetContent` pair, so
 * this module stays free of any network/React dependency (SN-8: the bundle is
 * exactly what the gateway returned, never recomputed here).
 */

import type { RunRecord } from "./recent-runs";

/** The on-disk export version (bump on a shape change). */
const EXPORT_VERSION = 1;

/** One committed output of a run: its producing Mote + the resolved content. */
export interface RunArtifactExport {
  readonly moteId: string;
  readonly resultRef: string;
  /** The decode kind (`text`/`json`/`binary`/`empty`) — see `content-decode`. */
  readonly kind: string;
  /** The displayable text (pretty JSON / raw text / bounded hex preview). */
  readonly text: string;
  readonly byteLength: number;
}

/** One committed Mote in the exported DAG (plain, serializable). */
export interface RunBundleMote {
  readonly moteId: string;
  readonly state: number;
  readonly ndClass: number;
  readonly committedSeq: number | null;
  readonly resultRef: string | null;
  readonly parents: ReadonlyArray<{
    readonly parentId: string;
    readonly edgeKind: string;
    readonly nonCascade: boolean;
  }>;
}

/** The committed projection + resolved artifacts for a rich run export. */
export interface RunBundle {
  readonly currentSeq: number;
  readonly motes: readonly RunBundleMote[];
  readonly artifacts: readonly RunArtifactExport[];
}

/** A safe, slugged filename for an exported run (never empty, no path chars). */
export function exportRunFilename(name: string, now: number = Date.now()): string {
  const slug =
    name
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || "run";
  return `kortecx-run-${slug}-${now}.json`;
}

/** Serialize a run (record + optional committed results) to a stable JSON string. */
export function exportRunJson(record: RunRecord, name: string, bundle?: RunBundle): string {
  const out = {
    kind: "kortecx.run",
    version: EXPORT_VERSION,
    name,
    instance_id: record.instanceId,
    terminal_mote_id: record.terminalMoteId,
    recipe_fingerprint: record.recipeFingerprint,
    handle: record.handle,
    args: record.args ?? null,
    started_at: record.startedAt,
    ...(bundle ? { results: bundle } : {}),
  };
  return JSON.stringify(out, null, 2);
}
