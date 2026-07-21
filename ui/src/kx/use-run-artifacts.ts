/**
 * The committed artifacts of one run — derived from its projection. Each committed
 * Mote carries a `result_ref`; this hook surfaces those `(moteId, resultRef)` pairs
 * so the gallery can fetch + decode each via the ownership-checked `GetContent`.
 *
 * Reuses `useProjection` (the same poll the run-detail DAG uses), so an in-flight
 * run's artifacts appear as its Motes commit, and polling stops once the run is at
 * rest. No new wire — pure derivation over the existing `GetProjection`.
 */

import { useMemo } from "react";
import { type ProjectionVM, useProjection } from "./use-projection";

/** One committed output of a run (its producing Mote + the content ref to fetch). */
export interface RunArtifact {
  readonly moteId: string;
  readonly resultRef: string;
}

export interface UseRunArtifacts {
  readonly artifacts: RunArtifact[];
  readonly projection: ProjectionVM | undefined;
  /** A scope anchor was supplied but is absent from the fold — the artifact list is
   *  EMPTY because we could not isolate the run, not because it produced nothing. */
  readonly scopeMissed: boolean;
  readonly isLoading: boolean;
  readonly error: unknown;
  refetch(): void;
}

/**
 * `scopeMoteId` MUST be threaded through, not defaulted away. It is part of
 * `useProjection`'s query key, so calling this hook unscoped is a genuinely different
 * cache entry from the run view's scoped one — which is how the graph came to show four
 * steps while the Artifacts tab beside it listed every output in the workspace.
 */
export function useRunArtifacts(
  instanceId: string | undefined,
  scopeMoteId?: string,
): UseRunArtifacts {
  const query = useProjection(instanceId, scopeMoteId ? { scopeMoteId } : {});
  const artifacts = useMemo<RunArtifact[]>(() => {
    const motes = query.data?.motes ?? [];
    return motes
      .filter((m): m is typeof m & { resultRef: string } => m.resultRef !== null)
      .map((m) => ({ moteId: m.moteId, resultRef: m.resultRef }));
  }, [query.data]);
  return {
    artifacts,
    projection: query.data,
    scopeMissed: query.data?.scopeMissed ?? false,
    isLoading: query.isLoading,
    error: query.error,
    refetch: () => void query.refetch(),
  };
}
