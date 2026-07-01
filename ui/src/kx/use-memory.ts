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

import type { DecayReport, Memory, MemoryHit, MemoryStats, StoreResult } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useMemories(includeTombstoned = false) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.memories(endpoint, includeTombstoned),
    enabled: status === "connected" && client !== null,
    retry: false,
    queryFn: async (): Promise<Memory[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listMemories({ includeTombstoned });
    },
  });
}

/** RC5b: namespace memory statistics (live/decayed counts, dim, fingerprint). */
export function useMemoryStats() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.memoryStats(endpoint),
    enabled: status === "connected" && client !== null,
    retry: false,
    queryFn: async (): Promise<MemoryStats> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.memoryStats();
    },
  });
}

/** RC5b: a decay PREVIEW (dry-run) for the given policy — evicts nothing. */
export function useMemoryDecay(ttlDays: number, minAccess: number, enabled: boolean) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.memoryDecay(endpoint, ttlDays, minAccess),
    enabled: enabled && status === "connected" && client !== null,
    retry: false,
    queryFn: async (): Promise<DecayReport> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.decayMemory({ ttlDays, minAccess, dryRun: true });
    },
  });
}

/** RC5b: APPLY a decay sweep (reversible soft-tombstones). */
export function useApplyDecay() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (policy: { ttlDays: number; minAccess: number }): Promise<DecayReport> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.decayMemory({ ...policy, dryRun: false });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["kx", endpoint, "memories"] });
      void qc.invalidateQueries({ queryKey: queryKeys.memoryStats(endpoint) });
    },
  });
}

/** RC5b: RESTORE (un-decay) a soft-tombstoned memory. */
export function useRestoreMemory() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (memoryId: string): Promise<boolean> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.restoreMemory(memoryId);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["kx", endpoint, "memories"] });
      void qc.invalidateQueries({ queryKey: queryKeys.memoryStats(endpoint) });
    },
  });
}

/** RC5b: CONSOLIDATE — drive a react-memory chain that distills episodics into one
 *  durable semantic fact (needs a served model). Returns the committed Result. */
export function useConsolidateMemory() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (opts: { query?: string; windowHours?: number }): Promise<{
      text?: string;
    }> => {
      if (!client) {
        throw new Error("not connected");
      }
      // dryRun:false always yields a committed Result (with a `.text` answer).
      return (await client.consolidateMemory({ ...opts, dryRun: false })) as { text?: string };
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["kx", endpoint, "memories"] });
      void qc.invalidateQueries({ queryKey: queryKeys.memoryStats(endpoint) });
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
