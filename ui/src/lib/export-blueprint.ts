/**
 * Serialize a Blueprint to a stable, self-describing JSON document (the
 * Blueprints card "Export" affordance). A pure transform over the
 * `GetRecipeForm` contract + the advisory catalog metadata
 * (description/tags/version). {@link blueprintInputs} is the single source of
 * the contract's input shape — reused by the read-only contract viewer so the
 * exported file and the on-screen contract never drift.
 */

import type { RecipeForm } from "@kortecx/sdk/web";

/** The on-disk export version (bump on a shape change). */
const EXPORT_VERSION = 1;

/** Advisory catalog metadata for a blueprint (display/discovery only, SN-8). */
export interface BlueprintMeta {
  readonly handle: string;
  readonly description?: string;
  readonly tags?: readonly string[];
  readonly version?: string;
}

/** One declared input of a blueprint's contract (plain, serializable). */
export interface BlueprintInput {
  readonly name: string;
  readonly type: string;
  readonly required: boolean;
  readonly max_len?: number;
  readonly allowed?: readonly string[];
}

/** Map a `RecipeForm` contract to its serializable input list (single source). */
export function blueprintInputs(form: RecipeForm): BlueprintInput[] {
  return form.fields.map((f) => ({
    name: f.name,
    type: f.type,
    required: f.required,
    ...(f.maxLen ? { max_len: f.maxLen } : {}),
    ...(f.allowed.length > 0 ? { allowed: f.allowed } : {}),
  }));
}

/** A safe, slugged filename for an exported blueprint (never empty, no path chars). */
export function exportBlueprintFilename(handle: string, now: number = Date.now()): string {
  const slug =
    handle
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || "blueprint";
  return `kortecx-blueprint-${slug}-${now}.json`;
}

/** Serialize a blueprint (metadata + contract inputs) to a stable JSON string. */
export function exportBlueprintJson(meta: BlueprintMeta, form: RecipeForm): string {
  const out = {
    kind: "kortecx.blueprint",
    version: EXPORT_VERSION,
    handle: meta.handle,
    description: meta.description ?? "",
    tags: meta.tags ?? [],
    blueprint_version: meta.version ?? "",
    inputs: blueprintInputs(form),
  };
  return JSON.stringify(out, null, 2);
}
