/**
 * Fetch + decode a committed artifact blob (`GetContent`). Content is
 * content-addressed (immutable), so the query never goes stale — it is cached
 * forever per (endpoint, instance, ref). The gateway enforces ownership server-side
 * (the ref must be a committed Mote's result in THIS run); we just render the bytes.
 */

import { useQuery } from "@tanstack/react-query";
import { type DecodedContent, decodeContent } from "../lib/content-decode";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useContent(instanceId: string | undefined, ref: string | undefined) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.content(endpoint, instanceId ?? "", ref ?? ""),
    enabled: status === "connected" && client !== null && Boolean(instanceId) && Boolean(ref),
    staleTime: Number.POSITIVE_INFINITY, // content-addressed ⇒ immutable
    queryFn: async (): Promise<DecodedContent> => {
      if (!client || !instanceId || !ref) {
        throw new Error("not connected");
      }
      const bytes = await client.getContent(ref, instanceId);
      return decodeContent(bytes);
    },
  });
}
