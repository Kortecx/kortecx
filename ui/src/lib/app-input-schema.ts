/**
 * POC-5d: map an App envelope's opaque `input_schema` (the documented subset
 * `{ fields: [{ name, type, required, allowed?, maxLen? }] }`) onto the existing
 * {@link RecipeForm} model so the App run drawer REUSES the typed `RecipeForm`
 * renderer + `lib/recipe-form` validator/builder. Unknown/missing ⇒ `null` (the App
 * runs with no inputs). Pure + total — unit-tested directly.
 */

import { RecipeForm, RecipeFormField, type RecipeParamTypeName } from "@kortecx/sdk/web";

const TYPES: ReadonlySet<string> = new Set(["str", "int", "bool", "bytes", "enum"]);

function fieldType(raw: unknown): RecipeParamTypeName {
  return typeof raw === "string" && TYPES.has(raw) ? (raw as RecipeParamTypeName) : "str";
}

/**
 * Build a {@link RecipeForm} from an App's `input_schema`, or `null` when there are
 * no usable input fields (the App takes no run inputs).
 */
export function appInputForm(handle: string, inputSchema: unknown): RecipeForm | null {
  if (inputSchema === null || typeof inputSchema !== "object") {
    return null;
  }
  const fieldsRaw = (inputSchema as { fields?: unknown }).fields;
  if (!Array.isArray(fieldsRaw)) {
    return null;
  }
  const fields = fieldsRaw
    .map((f): RecipeFormField | null => {
      if (f === null || typeof f !== "object") {
        return null;
      }
      const o = f as Record<string, unknown>;
      const name = typeof o.name === "string" ? o.name : "";
      if (name === "") {
        return null;
      }
      const required = o.required === true;
      const maxLen =
        typeof o.maxLen === "number" ? o.maxLen : typeof o.max_len === "number" ? o.max_len : null;
      const allowed = Array.isArray(o.allowed)
        ? o.allowed.filter((x): x is string => typeof x === "string")
        : [];
      return new RecipeFormField(name, fieldType(o.type), required, maxLen, allowed);
    })
    .filter((f): f is RecipeFormField => f !== null);
  return fields.length > 0 ? new RecipeForm(handle, fields) : null;
}
