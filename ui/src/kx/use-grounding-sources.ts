/**
 * Resolve a settled chat-rag turn's GROUNDING SOURCES — the exact chunk refs the
 * runtime folded into the answer Mote's identity-bearing
 * `config_subset["kx.context.items"]` — into displayable citations, entirely over
 * SHIPPED RPCs (`GetMoteDetail` → `GetContentBatch`). No new RPC: these are the
 * precise refs that grounded THIS answer (a different set ⇒ a different `MoteId`),
 * so the citations cannot drift from the reply. Commit-gated (`enabled`) so nothing
 * fires before the turn settles; content-addressed ⇒ cached forever. Display-only
 * (SN-8): no score is exposed (the wire carries none), and nothing authorizes.
 *
 * Honest-degrade: an ungrounded turn (empty/unknown dataset) has no such key ⇒ no
 * sources; an old gateway (`GetMoteDetail` UNIMPLEMENTED) soft-errors ⇒ no sources.
 * Either way the caller renders nothing — a plain answer never grows a faked cite.
 */

import { useQuery } from "@tanstack/react-query";
import { CONTEXT_ITEMS_KEY, type ContextItem, decodeContextItems } from "../lib/context-items";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";
import { useResultMap } from "./use-content-batch";

export interface GroundedSource {
  /** The 64-hex content ref of the grounded chunk (the citation key). */
  readonly ref: string;
  /** The advisory label the bind layer stored (display only). */
  readonly label: string;
  /** The decoded (preview-sized) chunk text — the grounding snippet. */
  readonly snippet: string;
  /** True iff the content store returned the uniform empty item. */
  readonly missing: boolean;
  /** True iff the snippet was cut at the per-item preview clamp. */
  readonly truncated: boolean;
}

export interface GroundingSources {
  readonly sources: readonly GroundedSource[];
  readonly loading: boolean;
  /** True iff the folded ref list overran the mote-detail value cap, so SOME
   *  grounded sources are omitted from `sources` (surfaced honestly, not hidden). */
  readonly truncated: boolean;
}

export function useGroundingSources(
  instanceId: string | undefined,
  moteId: string | undefined,
  enabled: boolean,
): GroundingSources {
  const { client, endpoint, status } = useConnection();
  const itemsQ = useQuery({
    queryKey: queryKeys.groundingSources(endpoint, instanceId ?? "none", moteId ?? "none"),
    enabled: enabled && status === "connected" && client !== null && !!instanceId && !!moteId,
    staleTime: Number.POSITIVE_INFINITY, // content-addressed answer ⇒ immutable
    queryFn: async (): Promise<{ items: readonly ContextItem[]; truncated: boolean }> => {
      if (!client || !instanceId || !moteId) {
        throw new Error("not connected");
      }
      const detail = await client.getMoteDetail(instanceId, moteId);
      const entry = detail.configSubset.find((e) => e.key === CONTEXT_ITEMS_KEY);
      // A truncated value means some folded refs were dropped by the detail cap —
      // carry the flag so the surface can say so rather than fake a complete list.
      return entry
        ? { items: decodeContextItems(entry.value), truncated: entry.truncated }
        : { items: [], truncated: false };
    },
  });

  const items = itemsQ.data?.items ?? [];
  const refs = items.map((it) => it.ref);
  // One batch round trip resolves every grounded ref → its preview snippet; the
  // content store denies a ref not committed by this run (run-scoped, SN-8).
  const { byRef, isLoading } = useResultMap(instanceId ?? "none", refs);

  const sources: GroundedSource[] = items.map((it) => {
    const vm = byRef.get(it.ref);
    return {
      ref: it.ref,
      label: it.label,
      snippet: vm?.content.text ?? "",
      missing: vm?.missing ?? false,
      truncated: vm?.truncated ?? false,
    };
  });

  return {
    sources,
    loading: itemsQ.isLoading || (refs.length > 0 && isLoading),
    truncated: itemsQ.data?.truncated ?? false,
  };
}
