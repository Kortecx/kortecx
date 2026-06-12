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
import { useRef } from "react";
import { isTerminalState } from "../lib/colors";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

const POLL_MS = 1000;

/** One inbound DAG edge (server-derived hex parent id + edge meta). */
export interface ParentEdgeVM {
  readonly parentId: string;
  readonly edgeKind: "data" | "control" | "unknown";
  readonly nonCascade: boolean;
}

/** A plain, serializable Mote view-model (no class methods → clean structural sharing). */
export interface MoteVM {
  readonly moteId: string;
  readonly stateCode: number;
  readonly ndClass: number;
  readonly promotion: number;
  readonly resultRef: string | null;
  readonly committedSeq: number | null;
  readonly anomaly: number | null;
  /** The committed def hash (hex); EMPTY until the Mote commits — the
   *  inspector's `GetMoteDetail` gate (PR-2). Off the DAG layout hash. */
  readonly moteDefHash: string;
  /** Inbound DAG edges — the source of the live graph's links (empty for a root). */
  readonly parents: readonly ParentEdgeVM[];
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
      moteDefHash: m.moteDefHash,
      parents: m.parents.map((e) => ({
        parentId: e.parentId,
        edgeKind: e.edgeKind,
        nonCascade: e.nonCascade,
      })),
    })),
  };
}

/** A run is "at rest" when it has Motes and they are all terminal (stop polling). */
export function allTerminal(p: ProjectionVM): boolean {
  return p.motes.length > 0 && p.motes.every((m) => isTerminalState(m.stateCode));
}

/**
 * Whether the run has settled — the cosmetic "live / at rest" signal. When the
 * recipe's terminal (sink) Mote id is known, it reaching a terminal state is
 * authoritative; otherwise fall back to "all visible Motes terminal".
 */
export function runSettled(p: ProjectionVM, terminalMoteId?: string): boolean {
  if (terminalMoteId) {
    return p.motes.some((m) => m.moteId === terminalMoteId && isTerminalState(m.stateCode));
  }
  return allTerminal(p);
}

/**
 * Whether polling can stop. The terminal (sink) Mote committing is authoritative —
 * it commits only AFTER the whole DAG does, so this is correct even while children
 * are still registering (incremental materialization / dynamic shaper children),
 * which a naive "all currently-visible Motes terminal" check gets wrong (it can
 * fire when only the root is present). Without a terminal id (a direct-URL nav),
 * fall back to a frontier-stability heuristic: every visible Mote terminal AND the
 * journal frontier (`current_seq`) did not advance this poll.
 */
export function isRunAtRest(
  data: ProjectionVM,
  terminalMoteId: string | undefined,
  prevSeq: number,
): boolean {
  if (terminalMoteId) {
    return runSettled(data, terminalMoteId);
  }
  return allTerminal(data) && data.currentSeq === prevSeq;
}

export interface UseProjectionOptions {
  atSeq?: number;
  /** The recipe's terminal (sink) Mote id — the authoritative run-complete signal. */
  terminalMoteId?: string;
}

export function useProjection(instanceId: string | undefined, opts: UseProjectionOptions = {}) {
  const { client, endpoint, status } = useConnection();
  const atSeq = opts.atSeq;
  const terminalMoteId = opts.terminalMoteId;
  // Tracks the journal frontier across polls for the fallback stop heuristic.
  const frontier = useRef<{ key: string; lastSeq: number }>({ key: "", lastSeq: -1 });
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
      const key = `${endpoint}|${instanceId ?? ""}`;
      const f = frontier.current;
      if (f.key !== key) {
        f.key = key; // a different run — reset frontier tracking
        f.lastSeq = -1;
      }
      const prevSeq = f.lastSeq;
      f.lastSeq = data.currentSeq;
      return isRunAtRest(data, terminalMoteId, prevSeq) ? false : POLL_MS;
    },
    refetchIntervalInBackground: false,
  });
}
