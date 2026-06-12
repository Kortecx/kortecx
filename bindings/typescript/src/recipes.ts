/**
 * The recipe-form view — a recipe's variable free-params, as enumerated by
 * `GetRecipeForm`, ready to render an input form. Kept in its own module so
 * `types.ts` stays a thin aggregator, mirroring the Rust core's
 * module-per-concern discipline.
 *
 * The param type renders to a stable lowercase name; an out-of-range value (a
 * future `RecipeParamType`) renders `"unspecified"` — never a crash, never a
 * silent mislabel (mirrors `stateName` / `edgeKindName`).
 */

import type {
  GetRecipeFormResponse as PbGetRecipeFormResponse,
  RecipeFormField as PbRecipeFormField,
} from "./gen/kortecx/v1/gateway_pb.js";
import { RecipeParamType } from "./gen/kortecx/v1/gateway_pb.js";

/** A free-param's value domain. `"unspecified"` absorbs UNSPECIFIED(0) + any new value. */
export type RecipeParamTypeName = "str" | "int" | "bool" | "bytes" | "enum" | "unspecified";

/** Map a `RecipeParamType` discriminant to a stable name (`"unspecified"` if new). */
export function recipeParamTypeName(t: number): RecipeParamTypeName {
  switch (t) {
    case RecipeParamType.STR:
      return "str";
    case RecipeParamType.INT:
      return "int";
    case RecipeParamType.BOOL:
      return "bool";
    case RecipeParamType.BYTES:
      return "bytes";
    case RecipeParamType.ENUM:
      return "enum";
    default:
      return "unspecified";
  }
}

/** One free-param a recipe requires (the unit a form renders as an input). */
export class RecipeFormField {
  constructor(
    readonly name: string,
    readonly type: RecipeParamTypeName,
    readonly required: boolean,
    /** Max length for `str` / `bytes` (else `null`). */
    readonly maxLen: number | null,
    /** Permitted values for `enum` (else empty). */
    readonly allowed: readonly string[],
  ) {}

  static fromProto(f: PbRecipeFormField): RecipeFormField {
    return new RecipeFormField(
      f.name,
      recipeParamTypeName(f.type),
      f.required,
      f.maxLen !== undefined ? Number(f.maxLen) : null,
      f.allowed,
    );
  }
}

/** A recipe's input FORM: its handle + the ordered variable free-param fields. */
export class RecipeForm {
  constructor(
    readonly handle: string,
    readonly fields: readonly RecipeFormField[],
  ) {}

  static fromProto(r: PbGetRecipeFormResponse): RecipeForm {
    return new RecipeForm(
      r.handle,
      r.fields.map((f) => RecipeFormField.fromProto(f)),
    );
  }
}

/*
 * Display-layer aliases (D136): the user-facing name is **Blueprint** — a
 * reusable, shareable workflow template. The WIRE stays the frozen `recipe`
 * vocabulary (`ListRecipes`/`GetRecipeForm`, `kx/recipes/*` handles), so these
 * are pure additive aliases; nothing is renamed or deprecated.
 */

/** Display alias for {@link RecipeForm} (wire: recipe). */
export const BlueprintForm = RecipeForm;
export type BlueprintForm = RecipeForm;

/** Display alias for {@link RecipeFormField} (wire: recipe). */
export const BlueprintFormField = RecipeFormField;
export type BlueprintFormField = RecipeFormField;

/** Display alias for {@link recipeParamTypeName} (wire: recipe). */
export const blueprintParamTypeName = recipeParamTypeName;
export type BlueprintParamTypeName = RecipeParamTypeName;

/** One catalog entry of `ListRecipes` (PR-2.1): the Invoke handle plus the
 *  published workflow fingerprint a bound run registers under — the join key
 *  for labeling durable `RunSummary` rows. Display/join only, never identity;
 *  `recipeFingerprint` is "" when the gateway predates the field. */
export class RecipeInfo {
  constructor(
    readonly handle: string,
    /** Hex fingerprint (joins `RunSummary.recipeFingerprint`); "" if unknown. */
    readonly recipeFingerprint: string,
  ) {}

  /** A plain snake_case object (the cross-SDK serialization shape). */
  toJSON() {
    return {
      handle: this.handle,
      recipe_fingerprint: this.recipeFingerprint,
    };
  }
}
