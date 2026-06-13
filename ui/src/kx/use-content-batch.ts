/**
 * Resolve a set of RUN-SCOPED content refs in ONE `GetContentBatch` round trip
 * (Batch A — the N+1 collapse), decoded for display. PR-2's Inputs pane uses
 * it to show each parent edge's RESOLVED text next to its digest (§4.11:
 * resolved text is the headline, the digest a pointer). Preview-sized fetches
 * (`maxBytesPerItem`) keep a wide fan-in affordable; `truncated` stays honest.
 * Content-addressed ⇒ the query never goes stale.
 */

import type { KxClientBase } from "@kortecx/sdk/web";
import { useQueries, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { type DecodedContent, decodeContent } from "../lib/content-decode";
import { chunkRefs } from "../lib/content-resolver";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export interface BatchedContentVM {
  /** The requested ref (hex), echoed in request order. */
  readonly contentRef: string;
  /** `true` iff the server returned the uniform empty item (no oracle). */
  readonly missing: boolean;
  /** `true` iff the payload was cut at the per-item clamp. */
  readonly truncated: boolean;
  /** The stored size (honest under truncation). */
  readonly fullSize: number;
  /** The decoded (possibly truncated) payload. */
  readonly content: DecodedContent;
}

const PREVIEW_BYTES = 4096n;

/** Resolve one run's refs (chunked at the 64-ref server cap). Run-scoped: the
 *  gateway denies a ref that is not a committed result of `instanceId`. */
async function fetchContentBatch(
  client: KxClientBase,
  instanceId: string,
  refs: readonly string[],
): Promise<BatchedContentVM[]> {
  const out: BatchedContentVM[] = [];
  for (const chunk of chunkRefs(refs)) {
    const items = await client.getContentBatch(chunk, {
      instanceId,
      maxBytesPerItem: PREVIEW_BYTES,
    });
    for (const i of items) {
      out.push({
        contentRef: i.contentRef,
        missing: i.missing,
        truncated: i.truncated,
        fullSize: Number(i.fullSize),
        content: decodeContent(i.payload),
      });
    }
  }
  return out;
}

export function useContentBatch(instanceId: string, refs: readonly string[]) {
  const { client, endpoint, status } = useConnection();
  // Refs are content-addressed and arrive in a stable (parent) order, so the
  // joined string is a stable cache key.
  const refsKey = refs.join(",");
  return useQuery({
    queryKey: queryKeys.contentBatch(endpoint, instanceId, refsKey),
    enabled: status === "connected" && client !== null && refs.length > 0,
    staleTime: Number.POSITIVE_INFINITY, // content-addressed ⇒ immutable
    queryFn: async (): Promise<BatchedContentVM[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return fetchContentBatch(client, instanceId, refs);
    },
  });
}

/**
 * Resolve a run's committed result refs and index them BY REF, for the
 * dense-context "resolved text headline + digest" surfaces (the mote table, the
 * DAG nodes, the artifact list, the event feeds). ONE batch round trip for all
 * visible refs (the N+1 collapse); content-addressed ⇒ cached forever. Returns
 * a lookup + `isLoading` so a row renders `resolving…` then its text.
 */
export function useResultMap(instanceId: string, refs: readonly string[]) {
  const q = useContentBatch(instanceId, refs);
  // Memoize on the query data so the map reference is STABLE across an unchanged
  // poll — feeding it into reactflow node data (the DAG) then never re-creates
  // nodes, preserving the no-thrash invariant.
  const byRef = useMemo(() => {
    const m = new Map<string, BatchedContentVM>();
    for (const vm of q.data ?? []) {
      m.set(vm.contentRef, vm);
    }
    return m;
  }, [q.data]);
  return { byRef, isLoading: q.isLoading };
}

/** One run's result ref, for the cross-run feed resolver. */
export interface RunScopedRef {
  readonly instanceId: string;
  readonly ref: string;
}

/**
 * Resolve result refs that span MULTIPLE runs (the global event feed). Because
 * `GetContentBatch` is run-scoped (a ref is fetchable only by its owning run),
 * the refs are grouped by `instanceId` and resolved with one batch query PER
 * RUN (`useQueries`). Returns a flat `byRef` lookup (content-addressed refs are
 * globally unique) + an aggregate `isLoading`. Content-addressed ⇒ cached
 * forever, so re-renders of the live tail never refetch a resolved row.
 */
export function useResultMapMulti(pairs: readonly RunScopedRef[]) {
  const { client, endpoint, status } = useConnection();
  // Group refs by run; dedupe within a run (a stable, sorted key per run).
  const groups = useMemo(() => {
    const m = new Map<string, Set<string>>();
    for (const { instanceId, ref } of pairs) {
      const set = m.get(instanceId);
      if (set) {
        set.add(ref);
      } else {
        m.set(instanceId, new Set([ref]));
      }
    }
    return [...m.entries()].map(([instanceId, set]) => ({
      instanceId,
      refs: [...set].sort(),
    }));
  }, [pairs]);

  // `combine` is memoized by React Query on the per-run results, so the merged
  // `byRef` reference is stable across renders until a batch actually settles
  // (a flat map — content-addressed refs are globally unique across runs).
  return useQueries({
    queries: groups.map(({ instanceId, refs }) => ({
      queryKey: queryKeys.contentBatch(endpoint, instanceId, refs.join(",")),
      enabled: status === "connected" && client !== null && refs.length > 0,
      staleTime: Number.POSITIVE_INFINITY,
      queryFn: async (): Promise<BatchedContentVM[]> => {
        if (!client) {
          throw new Error("not connected");
        }
        return fetchContentBatch(client, instanceId, refs);
      },
    })),
    combine: (results) => {
      const byRef = new Map<string, BatchedContentVM>();
      for (const r of results) {
        for (const vm of r.data ?? []) {
          byRef.set(vm.contentRef, vm);
        }
      }
      return { byRef, isLoading: results.some((r) => r.isLoading) };
    },
  });
}
