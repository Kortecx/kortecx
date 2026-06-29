/**
 * The Datasets data-plane (RAG) hooks: the corpora a gateway holds
 * (`ListDatasets`), a semantic query over one (`QueryDataset`), and a text-ingest
 * mutation (`IngestDocuments`). All tolerate a gateway that has not wired the
 * dataset view (UNIMPLEMENTED → the `hnsw` feature is off) by surfacing the error
 * for the section to degrade. Querying/ingesting TEXT needs a server embedder (the
 * `inference` feature); without one the gateway returns FAILED_PRECONDITION and the
 * panels show actionable guidance.
 */

import type { DatasetHit, DatasetSummary, IngestResult } from "@kortecx/sdk/web";
import { RetrievalMode } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useDatasets() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.datasets(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<DatasetSummary[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listDatasets();
    },
  });
}

export function useDatasetQuery(
  dataset: string | undefined,
  text: string,
  k = 10,
  mode: RetrievalMode = RetrievalMode.UNSPECIFIED,
) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    // RC4a: the retrieval mode is part of the cache key (dense vs hybrid differ).
    queryKey: [...queryKeys.datasetQuery(endpoint, dataset ?? "", text, k), mode],
    enabled:
      status === "connected" && client !== null && Boolean(dataset) && text.trim().length > 0,
    retry: false,
    queryFn: async (): Promise<DatasetHit[]> => {
      if (!client || !dataset) {
        throw new Error("not connected");
      }
      return client.queryDataset(dataset, { text, k, mode });
    },
  });
}

export interface IngestInput {
  readonly dataset: string;
  /** One document per entry (UTF-8 text; the server embeds it). */
  readonly docs: readonly string[];
}

export function useIngestDocuments() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ dataset, docs }: IngestInput): Promise<IngestResult> => {
      if (!client) {
        throw new Error("not connected");
      }
      const documents = docs.map((d) => ({ content: new TextEncoder().encode(d) }));
      return client.ingestDocuments(dataset, documents);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.datasets(endpoint) });
    },
  });
}
