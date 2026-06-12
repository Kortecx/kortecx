/**
 * Render a blueprint's free-param form (UI-2; `RecipeForm` on the frozen wire).
 * Pure presentation over the `lib/recipe-form` logic: each field is a typed input
 * (text / number / checkbox / enum select); on submit we validate + build the args
 * object and hand it up. The gateway re-validates server-side, so a stray value
 * surfaces as an Invoke error, not a silent bad run.
 */

import type { RecipeForm as RecipeFormDef, RecipeFormField } from "@kortecx/sdk/web";
import { type FormEvent, useState } from "react";
import { type FormValues, buildArgs, initialValues } from "../../lib/recipe-form";

function FieldInput({
  field,
  value,
  onChange,
}: {
  field: RecipeFormField;
  value: string;
  onChange: (v: string) => void;
}) {
  const id = `field-${field.name}`;
  if (field.type === "bool") {
    return (
      <input
        id={id}
        data-testid={id}
        type="checkbox"
        checked={value === "true"}
        onChange={(e) => onChange(e.target.checked ? "true" : "false")}
      />
    );
  }
  if (field.type === "enum") {
    return (
      <select id={id} data-testid={id} value={value} onChange={(e) => onChange(e.target.value)}>
        {field.allowed.map((opt) => (
          <option key={opt} value={opt}>
            {opt}
          </option>
        ))}
      </select>
    );
  }
  return (
    <input
      id={id}
      data-testid={id}
      type={field.type === "int" ? "number" : "text"}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      spellCheck={false}
      autoComplete="off"
    />
  );
}

export function RecipeForm({
  form,
  pending,
  onSubmit,
  initial,
}: {
  form: RecipeFormDef;
  pending: boolean;
  onSubmit: (args: Record<string, unknown>) => void;
  /** Prefill values (the PR-2.1 clone-lite landing: a run's prior args). Keys
   *  not in the form contract are ignored; the server re-validates at bind. */
  initial?: Record<string, unknown>;
}) {
  const [values, setValues] = useState<FormValues>(() => {
    const base = initialValues(form);
    if (initial) {
      for (const field of form.fields) {
        const v = initial[field.name];
        if (v !== undefined && v !== null) {
          base[field.name] = String(v);
        }
      }
    }
    return base;
  });
  const [errors, setErrors] = useState<Record<string, string>>({});

  function submit(e: FormEvent<HTMLFormElement>): void {
    e.preventDefault();
    const result = buildArgs(form, values);
    if (!result.ok) {
      setErrors(result.errors);
      return;
    }
    setErrors({});
    onSubmit(result.args);
  }

  return (
    <form
      className="invoke-form"
      data-testid="recipe-form"
      data-recipe={form.handle}
      onSubmit={submit}
    >
      {form.fields.length === 0 ? (
        <p className="muted">This blueprint takes no inputs — run it directly.</p>
      ) : (
        form.fields.map((field) => (
          <div className="form-field" key={field.name}>
            <label htmlFor={`field-${field.name}`}>
              {field.name}
              {field.required ? <span aria-hidden="true"> *</span> : null}
              <span className="muted form-field__type"> {field.type}</span>
            </label>
            <FieldInput
              field={field}
              value={values[field.name] ?? ""}
              onChange={(v) => setValues((prev) => ({ ...prev, [field.name]: v }))}
            />
            {errors[field.name] ? (
              <p className="field-error" role="alert">
                {errors[field.name]}
              </p>
            ) : null}
          </div>
        ))
      )}
      <button type="submit" disabled={pending}>
        {pending ? "Submitting…" : "Run blueprint"}
      </button>
    </form>
  );
}
