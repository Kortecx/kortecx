/**
 * Slice-B advisory fuzzy discovery over a dataset (`FuzzyDiscovery`, D151). The
 * fuzzy-in / exact-out primitive: it returns the ordered content-ref SET + a
 * DISPLAY-ONLY basis-point score (SN-8) — never content bytes. Like
 * {@link useDatasetQuery} it tolerates an unwired gateway (UNIMPLEMENTED → the
 * `hnsw` feature is off) by surfacing the error for the panel to degrade.
 */

import type { FuzzyHit } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useFuzzyDiscovery(dataset: string | undefined, text: string, k = 10) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.fuzzyDiscovery(endpoint, dataset ?? "", text, k),
    enabled:
      status === "connected" && client !== null && Boolean(dataset) && text.trim().length > 0,
    retry: false,
    queryFn: async (): Promise<FuzzyHit[]> => {
      if (!client || !dataset) {
        throw new Error("not connected");
      }
      return client.fuzzyDiscovery(dataset, { text, k });
    },
  });
}
