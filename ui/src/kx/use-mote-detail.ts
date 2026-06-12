/**
 * Resolve one Mote's admitted definition (`GetMoteDetail`, Batch B) for the
 * node-inspector panes. COMMIT-GATED by design: the def hash only exists on a
 * `Committed` fact, so the query is enabled only once `MoteVM.moteDefHash` is
 * non-empty — a pending node renders "available after commit" with NO RPC.
 * Content-addressed (the hash IS the blob's address) ⇒ the query never goes
 * stale. Display only (SN-8): nothing here authorizes anything.
 */

import type { MoteDetail } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

/** A plain view-model of the SDK {@link MoteDetail} (structural sharing). */
export interface MoteDetailVM {
  readonly defFound: boolean;
  readonly moteDefHash: string;
  readonly stepKind: string;
  readonly modelId: string;
  readonly prompt: string;
  readonly promptTruncated: boolean;
  readonly configSubset: readonly {
    readonly key: string;
    readonly value: Uint8Array;
    readonly truncated: boolean;
    readonly fullLen: number;
  }[];
  readonly toolContract: Readonly<Record<string, string>>;
  readonly logicRef: string;
  readonly ndClassName: string;
  readonly effectPatternName: string;
  readonly criticFor: string | undefined;
  readonly isTopologyShaper: boolean;
  readonly schemaVersion: number;
}

function toVM(d: MoteDetail): MoteDetailVM {
  return {
    defFound: d.defFound,
    moteDefHash: d.moteDefHash,
    stepKind: d.stepKind,
    modelId: d.modelId,
    prompt: d.prompt,
    promptTruncated: d.promptTruncated,
    configSubset: d.configSubset.map((e) => ({
      key: e.key,
      value: e.value,
      truncated: e.truncated,
      fullLen: e.fullLen,
    })),
    toolContract: d.toolContract,
    logicRef: d.logicRef,
    ndClassName: d.ndClassName,
    effectPatternName: d.effectPatternName,
    criticFor: d.criticFor,
    isTopologyShaper: d.isTopologyShaper,
    schemaVersion: d.schemaVersion,
  };
}

export function useMoteDetail(instanceId: string, moteId: string, defHash: string) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.moteDetail(endpoint, instanceId, moteId, defHash),
    enabled: status === "connected" && client !== null && defHash !== "",
    staleTime: Number.POSITIVE_INFINITY, // content-addressed def ⇒ immutable
    queryFn: async (): Promise<MoteDetailVM> => {
      if (!client) {
        throw new Error("not connected");
      }
      return toVM(await client.getMoteDetail(instanceId, moteId));
    },
  });
}
