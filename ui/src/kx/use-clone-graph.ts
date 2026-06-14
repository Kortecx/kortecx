/**
 * Clone-to-edit (D141.4) — reconstruct a builder graph from a COMMITTED run's DAG
 * so the user can remix it. Reads the run's projection (topology) + each Mote's
 * `GetMoteDetail` (kind / model / prompt / params), maps it to the builder's
 * constrained PURE / MODEL palette, and returns a `BuilderGraph`. The submit is
 * always a NEW workflow (new identity by construction — SN-8); the original run is
 * untouched. A non-PURE/MODEL Mote (shaper / critic / react-turn) maps to a PURE
 * step (the Tier-1 palette can't express it — stated honestly, not faked).
 */

import { useQuery } from "@tanstack/react-query";
import type {
  BuilderEdge,
  BuilderGraph,
  BuilderStep,
  BuilderStepKind,
} from "../components/builder/builder-graph";
import { useConnection } from "./connection-context";

export interface UseCloneGraph {
  readonly graph: BuilderGraph | undefined;
  readonly loading: boolean;
  readonly error: unknown;
}

/** Decode the committed `config_subset` into the builder's editable shape:
 *  the `prompt` key feeds the step prompt, `reasoning` feeds the chips, the rest
 *  become the params JSON object. Values are UTF-8 (un-JSON-quoted where they
 *  were JSON strings — the binder's canonical encoding). */
function decodeConfig(items: ReadonlyArray<{ key: string; value: Uint8Array }>): {
  prompt: string;
  reasoning: BuilderStep["reasoning"];
  paramsText: string;
} {
  let prompt = "";
  let reasoning: BuilderStep["reasoning"] = "";
  const params: Record<string, unknown> = {};
  const dec = new TextDecoder();
  for (const it of items) {
    const raw = dec.decode(it.value);
    let val: unknown = raw;
    try {
      val = JSON.parse(raw);
    } catch {
      /* keep the raw text */
    }
    if (it.key === "prompt") {
      prompt = typeof val === "string" ? val : raw;
    } else if (it.key === "reasoning") {
      const s = typeof val === "string" ? val : raw;
      reasoning = s === "full" || s === "minimal" || s === "off" ? s : "";
    } else {
      params[it.key] = val;
    }
  }
  return {
    prompt,
    reasoning,
    paramsText: Object.keys(params).length ? JSON.stringify(params, null, 2) : "",
  };
}

export function useCloneGraph(instanceId: string | null): UseCloneGraph {
  const { client, endpoint } = useConnection();
  const query = useQuery({
    queryKey: ["clone-graph", endpoint, instanceId],
    enabled: client !== null && instanceId !== null && instanceId.length > 0,
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
    queryFn: async (): Promise<BuilderGraph> => {
      if (!client || !instanceId) {
        throw new Error("not connected");
      }
      const proj = await client.getProjection(instanceId);
      const motes = proj.motes;
      // Builder-local ids in deterministic projection order (NOT MoteIds).
      const idOf = new Map<string, string>();
      motes.forEach((mv, i) => idOf.set(mv.moteId, `s${i}`));
      const details = await Promise.all(
        motes.map((mm) => client.getMoteDetail(instanceId, mm.moteId).catch(() => null)),
      );
      const steps: BuilderStep[] = motes.map((_mv, i) => {
        const d = details[i];
        const kind: BuilderStepKind = d?.stepKind === "model" ? "model" : "pure";
        const cfg = decodeConfig(d?.configSubset ?? []);
        return {
          id: `s${i}`,
          kind,
          label: kind === "model" ? "Agent" : "Step",
          modelId: d?.modelId ?? "",
          prompt: cfg.prompt,
          paramsText: cfg.paramsText,
          reasoning: cfg.reasoning,
        };
      });
      const edges: BuilderEdge[] = [];
      for (const mm of motes) {
        const target = idOf.get(mm.moteId);
        for (const p of mm.parents) {
          const source = idOf.get(p.parentId);
          if (source && target) {
            edges.push({
              id: `e-${source}-${target}`,
              source,
              target,
              edge: p.edgeKind === "control" ? "control" : "data",
              instruction: "",
            });
          }
        }
      }
      return { steps, edges };
    },
  });
  return { graph: query.data, loading: query.isLoading, error: query.error };
}
