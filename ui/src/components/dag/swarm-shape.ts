/**
 * Pure inference of a run's SWARM shape from the projection alone — no new RPC. A
 * swarm / supervisor / consensus is only a TOPOLOGY: parallel branches fanning into
 * one gather (reduce) sink. We detect it structurally over `MoteVM.parents[]`:
 *
 *   - a GATHER is a Mote with ≥2 present inbound `data` parents; those parents are
 *     its BRANCHES (the widest fan-in is the primary gather when several exist);
 *   - because content is content-addressed and a majority/pass-through reduce emits
 *     one branch's exact bytes, a branch WON iff its `resultRef` equals the gather's
 *     — so the majority winner + agreement count are RPC-free (no scores, SN-8);
 *   - the label stays HONEST: `consensus` only when ≥2 branches agreed on the
 *     emitted output; otherwise the neutral `parallel` (topology alone can't prove
 *     supervisor/swarm INTENT, so we never claim it).
 *
 * Kept pure + total (mirrors `dag-graph.ts`) so every shape is unit-tested without
 * rendering. Returns `null` for a run with no fan-in (a plain linear run shows no
 * swarm chrome — honest).
 */

import type { MoteVM } from "../../kx/use-projection";

export type SwarmPattern = "consensus" | "parallel";

export interface SwarmBranch {
  readonly moteId: string;
  readonly stateCode: number;
  /** The gather emitted this branch's exact output (a majority/pass-through winner). */
  readonly won: boolean;
}

export interface SwarmShape {
  readonly gatherId: string;
  readonly branches: readonly SwarmBranch[];
  readonly pattern: SwarmPattern;
  /** How many branches produced the exact output the gather emitted (agreement). */
  readonly agreementCount: number;
}

/**
 * Detect the primary swarm shape (the widest fan-in gather), or `null` when the run
 * has no ≥2-way `data` fan-in. Dangling parents (not present at this frontier) are
 * ignored, matching `dag-graph.buildEdges`.
 */
export function detectSwarm(motes: readonly MoteVM[]): SwarmShape | null {
  const present = new Set(motes.map((m) => m.moteId));
  const byId = new Map(motes.map((m) => [m.moteId, m] as const));

  let gather: MoteVM | null = null;
  let branchIds: string[] = [];
  for (const m of motes) {
    const dataParents = m.parents.filter((p) => p.edgeKind === "data" && present.has(p.parentId));
    if (dataParents.length >= 2 && dataParents.length > branchIds.length) {
      gather = m;
      branchIds = dataParents.map((p) => p.parentId);
    }
  }
  if (gather === null) {
    return null;
  }

  const gatherRef = gather.resultRef;
  const branches: SwarmBranch[] = branchIds.map((id) => {
    const b = byId.get(id);
    return {
      moteId: id,
      stateCode: b?.stateCode ?? 0,
      won: gatherRef !== null && b?.resultRef === gatherRef,
    };
  });
  const agreementCount = branches.filter((br) => br.won).length;
  const pattern: SwarmPattern = agreementCount >= 2 ? "consensus" : "parallel";
  return { gatherId: gather.moteId, branches, pattern, agreementCount };
}

/** The reactflow-edge ids (`parent->child`) of the branch→gather fan-in, for
 *  highlighting. Empty when there is no swarm shape. */
export function branchEdgeIds(shape: SwarmShape | null): ReadonlySet<string> {
  if (shape === null) {
    return new Set();
  }
  return new Set(shape.branches.map((b) => `${b.moteId}->${shape.gatherId}`));
}
