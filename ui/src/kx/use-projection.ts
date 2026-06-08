/**
 * The run-projection poll — the data layer's centerpiece and the T3.3 forward seam.
 * The run-detail view consumes only this hook; T3.3 swaps the *view* (table → DAG)
 * without touching the data layer. When T3.3 exposes `parents[]` on the SDK's
 * `MoteView`, `toProjectionVM` gains a `parents` field and nothing else changes.
 *
 * We poll `GetProjection` (unary gRPC-web) and stop once the run is at rest. We map
 * the SDK's `Projection` *class* into a PLAIN view-model so TanStack Query's
 * structural sharing keeps a stable reference across unchanged polls (memoized rows
 * then skip re-render).
 */

import type { Projection } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { isTerminalState } from "../lib/colors";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

const POLL_MS = 1000;

/** A plain, serializable Mote view-model (no class methods → clean structural sharing). */
export interface MoteVM {
  readonly moteId: string;
  readonly stateCode: number;
  readonly ndClass: number;
  readonly promotion: number;
  readonly resultRef: string | null;
  readonly committedSeq: number | null;
  readonly anomaly: number | null;
}

export interface ProjectionVM {
  readonly instanceId: string;
  readonly recipeFingerprint: string;
  readonly currentSeq: number;
  readonly motes: MoteVM[];
}

/** Map the SDK's `Projection` (class) to the plain VM the views consume. */
export function toProjectionVM(p: Projection): ProjectionVM {
  return {
    instanceId: p.instanceId,
    recipeFingerprint: p.recipeFingerprint,
    currentSeq: p.currentSeq,
    motes: p.motes.map((m) => ({
      moteId: m.moteId,
      stateCode: m.stateCode,
      ndClass: m.ndClass,
      promotion: m.promotion,
      resultRef: m.resultRef,
      committedSeq: m.committedSeq,
      anomaly: m.anomaly,
    })),
  };
}

/** A run is "at rest" when it has Motes and they are all terminal (stop polling). */
export function allTerminal(p: ProjectionVM): boolean {
  return p.motes.length > 0 && p.motes.every((m) => isTerminalState(m.stateCode));
}

export interface UseProjectionOptions {
  atSeq?: number;
}

export function useProjection(instanceId: string | undefined, opts: UseProjectionOptions = {}) {
  const { client, endpoint, status } = useConnection();
  const atSeq = opts.atSeq;
  return useQuery({
    queryKey: queryKeys.projection(endpoint, instanceId ?? "", atSeq),
    enabled: status === "connected" && client !== null && Boolean(instanceId),
    queryFn: async (): Promise<ProjectionVM> => {
      if (!client || !instanceId) {
        throw new Error("not connected");
      }
      const view = await client.getProjection(
        instanceId,
        atSeq != null ? { atSeq: BigInt(atSeq) } : {},
      );
      return toProjectionVM(view);
    },
    refetchInterval: (query) => {
      if (atSeq != null) {
        return false; // a pinned-seq snapshot is static
      }
      const data = query.state.data;
      if (!data) {
        return POLL_MS; // still loading
      }
      return allTerminal(data) ? false : POLL_MS; // stop once the run is at rest
    },
    refetchIntervalInBackground: false,
  });
}
