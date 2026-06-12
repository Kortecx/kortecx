/**
 * Re-resolve a RESTORED attachment's image preview through the uploads scope
 * (Batch A `GetContentBatch`). A live chat previews from the session-local
 * `blob:` URL of the user's own file; a thread restored from history has no
 * such URL, so the bytes come back from the gateway's content store (the SAME
 * server-derived ref) and become a fresh object URL. Content-addressed ⇒ the
 * query caches forever; a uniform-empty item (other endpoint / pruned store)
 * resolves to `null` and the bubble shows the chip only.
 */

import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { classifyItem } from "../lib/content-resolver";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useUploadPreview(ref: string, mediaType: string, enabled: boolean): string | null {
  const { client, endpoint, status } = useConnection();
  const query = useQuery({
    queryKey: queryKeys.contentBatch(endpoint, "uploads", ref),
    enabled: enabled && status === "connected" && client !== null && mediaType.startsWith("image/"),
    staleTime: Number.POSITIVE_INFINITY, // content-addressed ⇒ immutable
    retry: false,
    queryFn: async (): Promise<string | null> => {
      if (!client) {
        return null;
      }
      const items = await client.getContentBatch([ref]);
      const item = items[0];
      if (!item || item.missing) {
        return null;
      }
      const resolved = classifyItem(item);
      if (resolved.kind !== "image") {
        return null;
      }
      return URL.createObjectURL(new Blob([resolved.bytes as BlobPart], { type: mediaType }));
    },
  });

  const url = query.data ?? null;
  // Revoke the object URL when it falls out of use (component unmount or a
  // re-resolve) — mirrors use-attachments' lifecycle discipline.
  useEffect(() => {
    return () => {
      if (url !== null) {
        URL.revokeObjectURL(url);
      }
    };
  }, [url]);

  return url;
}
