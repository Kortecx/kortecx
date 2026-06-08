/**
 * Pure form logic for a recipe's free-param form (UI-2). Turns a `RecipeForm`
 * (the `GetRecipeForm` contract) into editable string values, validates each field
 * against its declared type, and builds the JSON args object `Invoke` expects.
 *
 * No React: the component renders + collects strings, this module owns the
 * type/validation/coercion. The gateway re-validates server-side (fail-closed),
 * so this is an ergonomic first line, never the authority (SN-8).
 */

import type { RecipeForm, RecipeFormField } from "@kortecx/sdk/web";

/** Raw editable values, keyed by field name (everything is a string in the DOM;
 *  a boolean field stores `"true"`/`"false"`). */
export type FormValues = Record<string, string>;

/** Fresh empty values for a form — booleans default to `"false"`, enums to their
 *  first allowed value, everything else to `""`. */
export function initialValues(form: RecipeForm): FormValues {
  const out: FormValues = {};
  for (const f of form.fields) {
    if (f.type === "bool") {
      out[f.name] = "false";
    } else if (f.type === "enum") {
      out[f.name] = f.allowed[0] ?? "";
    } else {
      out[f.name] = "";
    }
  }
  return out;
}

/** Validate one field's raw value; return a human message, or `null` if valid. */
export function validateField(field: RecipeFormField, raw: string): string | null {
  const v = raw.trim();
  if (field.type === "bool") {
    return v === "true" || v === "false" ? null : "must be true or false";
  }
  if (v === "") {
    return field.required ? "required" : null;
  }
  switch (field.type) {
    case "int": {
      if (!/^-?\d+$/.test(v)) {
        return "must be a whole number";
      }
      return null;
    }
    case "enum": {
      return field.allowed.includes(v) ? null : `must be one of: ${field.allowed.join(", ")}`;
    }
    case "str":
    case "bytes": {
      if (field.maxLen !== null && v.length > field.maxLen) {
        return `at most ${field.maxLen} characters`;
      }
      return null;
    }
    default: {
      // "unspecified" — accept any non-empty value (the server is the authority).
      return null;
    }
  }
}

/** Coerce a validated raw value to the JSON type the recipe expects. */
function coerce(field: RecipeFormField, raw: string): unknown {
  const v = raw.trim();
  switch (field.type) {
    case "int":
      return Number.parseInt(v, 10);
    case "bool":
      return v === "true";
    default:
      // str / bytes / enum / unspecified → a JSON string.
      return raw;
  }
}

export type BuildResult =
  | { readonly ok: true; readonly args: Record<string, unknown> }
  | { readonly ok: false; readonly errors: Record<string, string> };

/**
 * Validate every field and, on success, build the args object. An optional field
 * left blank is omitted (the recipe keeps its default); a required field must be
 * present.
 */
export function buildArgs(form: RecipeForm, values: FormValues): BuildResult {
  const errors: Record<string, string> = {};
  const args: Record<string, unknown> = {};
  for (const field of form.fields) {
    const raw = values[field.name] ?? "";
    const err = validateField(field, raw);
    if (err !== null) {
      errors[field.name] = err;
      continue;
    }
    // Omit a blank optional non-bool field (let the recipe default apply).
    if (raw.trim() === "" && field.type !== "bool" && !field.required) {
      continue;
    }
    args[field.name] = coerce(field, raw);
  }
  if (Object.keys(errors).length > 0) {
    return { ok: false, errors };
  }
  return { ok: true, args };
}
