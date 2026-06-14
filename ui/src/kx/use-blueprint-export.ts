/**
 * Export a Blueprint's definition as JSON (the Blueprints card affordance).
 * Fetches the `GetRecipeForm` contract on demand and serializes it (with the
 * advisory catalog metadata) via the pure `lib/export-blueprint`. The fetch is
 * on-demand — NOT a per-card query — so the catalog grid stays light.
 */

import { useState } from "react";
import { download } from "../lib/download";
import {
  type BlueprintMeta,
  exportBlueprintFilename,
  exportBlueprintJson,
} from "../lib/export-blueprint";
import { useConnection } from "./connection-context";

export interface UseBlueprintExport {
  exportBlueprint(meta: BlueprintMeta): Promise<void>;
  /** The handle currently fetching its contract, or `null`. */
  readonly pendingHandle: string | null;
  readonly error: unknown;
}

export function useBlueprintExport(): UseBlueprintExport {
  const { client } = useConnection();
  const [pendingHandle, setPendingHandle] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);

  async function exportBlueprint(meta: BlueprintMeta): Promise<void> {
    if (!client) {
      return;
    }
    setPendingHandle(meta.handle);
    setError(null);
    try {
      const form = await client.getRecipeForm(meta.handle);
      download(
        exportBlueprintFilename(meta.handle),
        exportBlueprintJson(meta, form),
        "application/json",
      );
    } catch (e) {
      setError(e);
    } finally {
      setPendingHandle(null);
    }
  }

  return { exportBlueprint, pendingHandle, error };
}
