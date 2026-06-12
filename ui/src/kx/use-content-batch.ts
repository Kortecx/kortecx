/**
 * Resolve a set of RUN-SCOPED content refs in ONE `GetContentBatch` round trip
 * (Batch A — the N+1 collapse), decoded for display. PR-2's Inputs pane uses
 * it to show each parent edge's RESOLVED text next to its digest (§4.11:
 * resolved text is the headline, the digest a pointer). Preview-sized fetches
 * (`maxBytesPerItem`) keep a wide fan-in affordable; `truncated` stays honest.
 * Content-addressed ⇒ the query never goes stale.
 */

import { useQuery } from "@tanstack/react-query";
import { type DecodedContent, decodeContent } from "../lib/content-decode";
import { chunkRefs } from "../lib/content-resolver";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export interface BatchedContentVM {
  /** The requested ref (hex), echoed in request order. */
  readonly contentRef: string;
  /** `true` iff the server returned the uniform empty item (no oracle). */
  readonly missing: boolean;
  /** `true` iff the payload was cut at the per-item clamp. */
  readonly truncated: boolean;
  /** The stored size (honest under truncation). */
  readonly fullSize: number;
  /** The decoded (possibly truncated) payload. */
  readonly content: DecodedContent;
}

const PREVIEW_BYTES = 4096n;

export function useContentBatch(instanceId: string, refs: readonly string[]) {
  const { client, endpoint, status } = useConnection();
  // Refs are content-addressed and arrive in a stable (parent) order, so the
  // joined string is a stable cache key.
  const refsKey = refs.join(",");
  return useQuery({
    queryKey: queryKeys.contentBatch(endpoint, instanceId, refsKey),
    enabled: status === "connected" && client !== null && refs.length > 0,
    staleTime: Number.POSITIVE_INFINITY, // content-addressed ⇒ immutable
    queryFn: async (): Promise<BatchedContentVM[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      // Chunk at the server's 64-ref cap (a >64 fan-in stays correct).
      const out: BatchedContentVM[] = [];
      for (const chunk of chunkRefs(refs)) {
        const items = await client.getContentBatch(chunk, {
          instanceId,
          maxBytesPerItem: PREVIEW_BYTES,
        });
        for (const i of items) {
          out.push({
            contentRef: i.contentRef,
            missing: i.missing,
            truncated: i.truncated,
            fullSize: Number(i.fullSize),
            content: decodeContent(i.payload),
          });
        }
      }
      return out;
    },
  });
}
