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
  RecipeSummary as PbRecipeSummary,
  ScoredRecipe as PbScoredRecipe,
} from "./gen/kortecx/v1/gateway_pb.js";
import { RecipeParamType } from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

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
 *  for labeling durable `RunSummary` rows. PR-4 Batch D adds the ADVISORY
 *  metadata (description / tags / version) — display/discovery ONLY, never
 *  identity, never enforcement. `recipeFingerprint` / metadata are empty when
 *  the gateway predates the field. */
export class RecipeInfo {
  constructor(
    readonly handle: string,
    /** Hex fingerprint (joins `RunSummary.recipeFingerprint`); "" if unknown. */
    readonly recipeFingerprint: string,
    /** Free-form advisory description (never parsed for enforcement); "" if unknown. */
    readonly description: string = "",
    /** Advisory discovery tags; empty if unknown. */
    readonly tags: readonly string[] = [],
    /** Advisory published version label; "" if unversioned/unknown. */
    readonly version: string = "",
  ) {}

  static fromProto(r: PbRecipeSummary): RecipeInfo {
    return new RecipeInfo(
      r.handle,
      encode(r.recipeFingerprint),
      r.description,
      r.tags,
      r.version,
    );
  }

  /** A plain snake_case object (the cross-SDK serialization shape). */
  toJSON() {
    return {
      handle: this.handle,
      recipe_fingerprint: this.recipeFingerprint,
      description: this.description,
      tags: this.tags,
      version: this.version,
    };
  }
}

/** One ranked `SearchRecipes` hit (PR-4 Batch D): the matched recipe plus its
 *  advisory rank in integer basis points (0..=10000). SN-8: `scoreBp` is
 *  DISPLAY-ONLY — a search SURFACES a recipe, never invokes one (`Invoke` stays
 *  the authorization gate). */
export class ScoredRecipe {
  constructor(
    readonly recipe: RecipeInfo,
    /** Advisory rank, integer basis points (0..=10000); never a float. */
    readonly scoreBp: number,
  ) {}

  static fromProto(s: PbScoredRecipe): ScoredRecipe {
    return new ScoredRecipe(
      s.recipe ? RecipeInfo.fromProto(s.recipe) : new RecipeInfo("", ""),
      s.scoreBp,
    );
  }

  toJSON() {
    return { recipe: this.recipe.toJSON(), score_bp: this.scoreBp };
  }
}
