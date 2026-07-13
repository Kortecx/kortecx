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
  rerank?: boolean,
) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    // RC4a/RC4c: the retrieval mode AND the rerank override are part of the cache key
    // (dense vs hybrid, and MMR on/off/default, all differ).
    queryKey: [...queryKeys.datasetQuery(endpoint, dataset ?? "", text, k), mode, rerank ?? "auto"],
    enabled:
      status === "connected" && client !== null && Boolean(dataset) && text.trim().length > 0,
    retry: false,
    queryFn: async (): Promise<DatasetHit[]> => {
      if (!client || !dataset) {
        throw new Error("not connected");
      }
      return client.queryDataset(dataset, { text, k, mode, rerank });
    },
  });
}

/** A pre-read binary document — raw file bytes + advisory metadata (name/media type).
 *  `metadata` is forward-compat (accepted on the wire, not yet persisted; SN-8) — set
 *  so a later artifact-preview decode can hint off the name/media type. */
export interface FileDoc {
  readonly content: Uint8Array;
  readonly metadata?: Readonly<Record<string, string>>;
}

export interface IngestInput {
  readonly dataset: string;
  /** Text documents — one per entry (UTF-8; the server embeds each). */
  readonly docs?: readonly string[];
  /** Binary documents — already-read file bytes (multimodal ingest). */
  readonly fileDocs?: readonly FileDoc[];
}

export function useIngestDocuments() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({
      dataset,
      docs = [],
      fileDocs = [],
    }: IngestInput): Promise<IngestResult> => {
      if (!client) {
        throw new Error("not connected");
      }
      // Text docs are UTF-8 encoded; file docs already carry raw bytes. Both cross the
      // wire as `IngestDoc.content: Uint8Array` — the SAME shipped `IngestDocuments` RPC.
      const documents = [
        ...docs.map((d) => ({ content: new TextEncoder().encode(d) })),
        ...fileDocs,
      ];
      return client.ingestDocuments(dataset, documents);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.datasets(endpoint) });
    },
  });
}
