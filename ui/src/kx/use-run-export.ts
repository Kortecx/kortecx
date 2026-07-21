/**
 * Export a run as JSON (the Workflows card affordance). Two paths over ONE pure
 * envelope (`lib/export-run`):
 *   - `exportLight` — synchronous, the client-known {@link RunRecord} only.
 *   - `exportRich`  — fetches the committed projection (`GetProjection`) + each
 *     Mote's resolved output (`GetContent`), proving the UI↔gateway dataflow.
 * The impure fetch/decode lives here; the serialization stays pure + tested in
 * `lib/export-run`. `pendingId` is the instance currently fetching (one card at
 * a time per hook instance); `error` is the last rich-export failure.
 */

import { useState } from "react";
import { decodeContent } from "../lib/content-decode";
import { download } from "../lib/download";
import {
  type RunArtifactExport,
  type RunBundle,
  exportRunFilename,
  exportRunJson,
} from "../lib/export-run";
import type { RunRecord } from "../lib/recent-runs";
import { runAnchor } from "../lib/run-anchor";
import { useConnection } from "./connection-context";
import { scopeProjection, toProjectionVM } from "./use-projection";

const MIME = "application/json";

export interface UseRunExport {
  exportLight(record: RunRecord, name: string): void;
  exportRich(record: RunRecord, name: string): Promise<void>;
  /** The instanceId currently fetching a rich export, or `null`. */
  readonly pendingId: string | null;
  readonly error: unknown;
}

export function useRunExport(): UseRunExport {
  const { client } = useConnection();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);

  function exportLight(record: RunRecord, name: string): void {
    download(exportRunFilename(name), exportRunJson(record, name), MIME);
  }

  async function exportRich(record: RunRecord, name: string): Promise<void> {
    if (!client) {
      return;
    }
    setPendingId(record.instanceId);
    setError(null);
    try {
      // SCOPE the fold to this run before walking it. Unscoped, a "run export" on a
      // long-lived serve is an export of the entire journal — every other run's Motes AND
      // a `GetContent` per foreign artifact, so the file is both wrong and slow. The
      // record carries the anchors; an unscopable record (a durable-only row) exports the
      // journal as before rather than silently producing an EMPTY bundle.
      const anchor = runAnchor(record);
      const scoped = scopeProjection(
        toProjectionVM(await client.getProjection(record.instanceId)),
        anchor || undefined,
      );
      if (scoped.scopeMissed) {
        // The anchor is not in the fold (a stale record, a rebuilt journal). A miss leaves
        // `motes` UNSCOPED, so falling through would silently write the entire workspace
        // to disk under this run's filename. Refuse; `error` surfaces it on the card.
        throw new Error("this run's steps are not in the gateway's journal — nothing to export");
      }
      const projection = scoped;
      const artifacts: RunArtifactExport[] = [];
      for (const motes of projection.motes) {
        if (motes.resultRef === null) {
          continue;
        }
        const decoded = decodeContent(await client.getContent(motes.resultRef, record.instanceId));
        artifacts.push({
          moteId: motes.moteId,
          resultRef: motes.resultRef,
          kind: decoded.kind,
          text: decoded.text,
          byteLength: decoded.byteLength,
        });
      }
      const bundle: RunBundle = {
        currentSeq: projection.currentSeq,
        motes: projection.motes.map((m) => ({
          moteId: m.moteId,
          state: m.stateCode,
          ndClass: m.ndClass,
          committedSeq: m.committedSeq,
          resultRef: m.resultRef,
          parents: m.parents.map((e) => ({
            parentId: e.parentId,
            edgeKind: e.edgeKind,
            nonCascade: e.nonCascade,
          })),
        })),
        artifacts,
      };
      download(exportRunFilename(name), exportRunJson(record, name, bundle), MIME);
    } catch (e) {
      setError(e);
    } finally {
      setPendingId(null);
    }
  }

  return { exportLight, exportRich, pendingId, error };
}
