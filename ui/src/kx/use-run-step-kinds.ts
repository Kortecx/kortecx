/**
 * PR-D: resolve each COMMITTED Mote's high-level step type (model / MCP / connector
 * / tool / action) for the read-only run review — one `GetMoteDetail` per node via
 * `useQueries`, keyed by the SAME `moteDetail` cache key + value shape the inspector
 * uses, so a click never refetches. Commit-gated (a def hash only exists on a
 * Committed fact) + content-addressed ⇒ cached forever. Display only.
 */

import { useQueries } from "@tanstack/react-query";
import { useRef } from "react";
import { type StepType, classifyStep } from "../lib/step-kind";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";
import { moteDetailToVM } from "./use-mote-detail";
import type { MoteVM } from "./use-projection";

/** A stable, per-endpoint map of moteId → its high-level step type. Motes that are
 *  not yet committed (no def hash) are absent until they commit. */
export function useRunStepKinds(
  instanceId: string,
  motes: readonly MoteVM[],
): ReadonlyMap<string, StepType> {
  const { client, endpoint, status } = useConnection();
  const committed = motes.filter((m) => m.moteDefHash !== "");
  // ⚠ `combine` runs on EVERY render and builds a fresh Map, so the value below is a new
  // reference each time even when nothing changed — which this hook's contract ("a stable
  // map") always claimed it was not. That mattered once the DAG began subscribing to
  // reactflow's store: an unstable node-data input re-created the `nodes` prop, reactflow
  // re-adopted every node, re-adoption drops each node's `measured` size, and a node with
  // no measured size is rendered `visibility: hidden`. The canvas kept its node count and
  // its text — so counts and content assertions passed — while nothing on it could be
  // CLICKED. Latching the identity to the map's CONTENT is what makes the graph settle.
  const stable = useRef<ReadonlyMap<string, StepType>>(new Map());
  const stableKey = useRef("");
  const kinds = useQueries({
    queries: committed.map((m) => ({
      queryKey: queryKeys.moteDetail(endpoint, instanceId, m.moteId, m.moteDefHash),
      enabled: status === "connected" && client !== null,
      staleTime: Number.POSITIVE_INFINITY, // committed def ⇒ immutable
      queryFn: async () => {
        if (!client) {
          throw new Error("not connected");
        }
        // Return the SAME MoteDetailVM the inspector caches under this key.
        return moteDetailToVM(await client.getMoteDetail(instanceId, m.moteId));
      },
    })),
    combine: (results) => {
      const map = new Map<string, StepType>();
      results.forEach((r, i) => {
        const mote = committed[i];
        if (r.data && mote) {
          map.set(mote.moteId, classifyStep(r.data.stepKind, r.data.toolContract));
        }
      });
      return map;
    },
  });
  const key = [...kinds]
    .map(([id, k]) => `${id}:${k}`)
    .sort()
    .join(",");
  if (key !== stableKey.current) {
    stableKey.current = key;
    stable.current = kinds;
  }
  return stable.current;
}
