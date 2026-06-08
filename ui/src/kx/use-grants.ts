/**
 * The sharing (grants) inspector: every grant on an asset, fold-classified
 * root/delegated + active/revoked (`ListAssetGrants`). Tolerates a gateway that has
 * not wired the grant view (UNIMPLEMENTED → the query errors; the inspector degrades
 * to a not-wired empty state) and an unknown asset (NOT_FOUND). VIEW-only in OSS.
 */

import type { AssetGrants } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useAssetGrants(assetRef: string | undefined) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.assetGrants(endpoint, assetRef ?? ""),
    enabled: status === "connected" && client !== null && Boolean(assetRef),
    queryFn: async (): Promise<AssetGrants> => {
      if (!client || !assetRef) {
        throw new Error("not connected");
      }
      return client.listAssetGrants(assetRef);
    },
  });
}
