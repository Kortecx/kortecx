/**
 * The App capability-manifest hook (`GetAppManifest`) — "what this App needs vs. what
 * you have": its requested tools / connections / model diffed against your live policy
 * (fireable tools, registered connections, served models). Server-authoritative (the
 * gateway reuses the same policy folds `RunApp` applies), so the panel can never claim
 * a capability in-policy that a run would drop. Advisory: it gates nothing.
 *
 * On an OLDER gateway without the seam (`Unimplemented` ⇒ `not-wired`), the hook
 * degrades to a NEEDS-ONLY view derived from the App envelope alone (the declared
 * tools/connections/model, no have/missing verdict), so the panel stays useful.
 */

import type { AppManifest } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";
import { useApp } from "./use-apps";

/** One normalized manifest row (tool or connection). */
export interface ManifestRow {
  readonly id: string;
  readonly version: string;
  readonly requested: boolean;
  readonly inPolicy: boolean;
  readonly inherited: boolean;
}

/** The normalized manifest the panel renders (server-resolved or needs-only). */
export interface ManifestView {
  readonly reachInherit: boolean;
  readonly tools: ManifestRow[];
  readonly connections: ManifestRow[];
  readonly modelRoute: string;
  readonly modelRouteServed: boolean;
  /** True when derived from the envelope alone (the server could not resolve policy). */
  readonly needsOnly: boolean;
}

function fromManifest(m: AppManifest): ManifestView {
  return {
    reachInherit: m.reachInherit,
    tools: m.tools,
    connections: m.connections,
    modelRoute: m.modelRoute,
    modelRouteServed: m.modelRouteServed,
    needsOnly: false,
  };
}

/** Derive a NEEDS-ONLY view from the raw envelope (the older-gateway fallback). */
function deriveNeeds(envelope: Record<string, unknown>): ManifestView {
  const steering = (envelope.steering_config ?? {}) as Record<string, unknown>;
  const toolsCfg = (steering.tools ?? {}) as Record<string, unknown>;
  const grants = (toolsCfg.requested_grants ?? {}) as Record<string, string>;
  const modelCfg = (steering.model ?? {}) as Record<string, unknown>;
  const refs = (envelope.references ?? {}) as Record<string, unknown>;
  const conns = (refs.connections ?? []) as { descriptor?: string }[];
  return {
    reachInherit: toolsCfg.reach === "inherit_principal",
    tools: Object.entries(grants).map(([id, version]) => ({
      id,
      version,
      requested: true,
      inPolicy: false,
      inherited: false,
    })),
    connections: conns.map((c) => ({
      id: c.descriptor ?? "",
      version: "",
      requested: true,
      inPolicy: false,
      inherited: false,
    })),
    modelRoute: typeof modelCfg.model_route === "string" ? modelCfg.model_route : "",
    modelRouteServed: false,
    needsOnly: true,
  };
}

export function useAppManifest(handle: string | null) {
  const { client, endpoint, status } = useConnection();
  const app = useApp(handle); // for the needs-only fallback on an older gateway
  const q = useQuery({
    queryKey: queryKeys.appManifest(endpoint, handle ?? ""),
    enabled: status === "connected" && client !== null && handle !== null,
    // The seam is either present or not — don't retry an Unimplemented gateway.
    retry: false,
    queryFn: async (): Promise<AppManifest | null> => {
      if (!client || handle === null) {
        throw new Error("not connected");
      }
      return client.getAppManifest(handle);
    },
  });

  const notWired = q.isError && toUiError(q.error).kind === "not-wired";
  let view: ManifestView | null = null;
  if (q.data) {
    view = fromManifest(q.data);
  } else if (notWired && app.data) {
    view = deriveNeeds(app.data.envelope);
  }

  return {
    view,
    // `null` (not found / not owned) is a distinct, honest empty state.
    notFound: q.data === null && !notWired,
    isLoading: q.isLoading || (notWired && app.isLoading),
    // A not-wired seam is handled by the fallback, not surfaced as an error.
    error: notWired ? null : q.error,
  };
}
