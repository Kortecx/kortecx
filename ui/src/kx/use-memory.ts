/**
 * RC5a durable agentic MEMORY hooks: the episodic log a gateway holds
 * (`ListMemories`), a semantic recall over it (`RecallMemory`), a store mutation
 * (`StoreMemory`), and a forget mutation (`ForgetMemory`). All tolerate a gateway
 * that has not enabled memory (UNIMPLEMENTED → `KX_SERVE_MEMORY` off / no `hnsw`) by
 * surfacing the error for the section to degrade. Storing/recalling TEXT needs a
 * server embedder (the `inference` feature); without one the gateway returns
 * FAILED_PRECONDITION and the panels show actionable guidance. Every memory is scoped
 * to the caller's own principal (server-derived).
 */

import type { Memory, MemoryHit, StoreResult } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useMemories() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.memories(endpoint),
    enabled: status === "connected" && client !== null,
    retry: false,
    queryFn: async (): Promise<Memory[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listMemories();
    },
  });
}

export function useMemoryRecall(text: string, k = 5) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.memoryRecall(endpoint, text, k),
    enabled: status === "connected" && client !== null && text.trim().length > 0,
    retry: false,
    queryFn: async (): Promise<MemoryHit[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.recallMemory(text, { k });
    },
  });
}

export function useStoreMemory() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (content: string): Promise<StoreResult> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.storeMemory(content);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.memories(endpoint) });
    },
  });
}

export function useForgetMemory() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (memoryId: string): Promise<boolean> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.forgetMemory(memoryId);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.memories(endpoint) });
    },
  });
}
