/**
 * The published recipe catalog (read-only). Tolerant of an empty catalog and of a
 * gateway that has not wired the catalog read path (UNIMPLEMENTED → the query errors
 * and the UI shows a "not available here" notice; the rest of the shell stays usable).
 */

import type { SignatureSummary } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useSignatures() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.signatures(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<SignatureSummary[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listSignatures();
    },
  });
}
