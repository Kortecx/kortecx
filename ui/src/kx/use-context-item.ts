/**
 * Fetch + decode a context-item's FULL body (POC-2 view/edit). A context item is
 * an UPLOADS-scope content ref (authored via `PutContent` / `kx context add`), so
 * it reads through the single `GetContent` uploads path (`instanceId` omitted) —
 * which returns the WHOLE payload, uncapped. This deliberately does NOT reuse the
 * `use-content-batch` `PREVIEW_BYTES` (4096) clamp: an edit must seed Monaco with
 * the complete body, never a truncated preview. Content-addressed ⇒ the query
 * caches forever per (endpoint, ref); enable it only when the row is expanded so a
 * wide bundle never N+1-fetches every item body on render.
 */

import { useQuery } from "@tanstack/react-query";
import { type DecodedContent, decodeContent } from "../lib/content-decode";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useContextItemBody(
  contentRef: string,
  mediaType: string,
  name: string,
  enabled: boolean,
) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.contextItemBody(endpoint, contentRef),
    enabled: enabled && status === "connected" && client !== null,
    staleTime: Number.POSITIVE_INFINITY, // content-addressed ⇒ immutable
    retry: false,
    queryFn: async (): Promise<DecodedContent> => {
      if (!client) {
        throw new Error("not connected");
      }
      const bytes = await client.getContent(contentRef); // uploads scope, FULL bytes
      return decodeContent(bytes, { mediaType, filename: name });
    },
  });
}
