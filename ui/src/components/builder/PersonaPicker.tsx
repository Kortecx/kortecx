/**
 * The persona picker for a step's instruction.
 *
 * A persona is a curated role whose text is PREPENDED to the step's prompt, so it
 * is identity-bearing: the same persona + task always compile to the same recipe
 * fingerprint. That makes "which persona is on this step" a real piece of state —
 * and the drawer's original inline chips never showed it. They had no
 * `chip--active` and no `aria-pressed`, so the row looked identical whether a
 * persona was applied or not, and there was no way to take one off again.
 *
 * The swap logic lives here as PURE functions ({@link activePersona},
 * {@link withPersona}) rather than inline in a click handler, because it is the
 * part that can be wrong in a way nobody sees — re-picking must SWAP the role, not
 * stack a second one onto the prompt.
 */

import { PERSONAS, personaNames } from "@kortecx/sdk/web";
import { useState } from "react";

/**
 * The persona a prompt currently leads with, or `null` for none.
 *
 * Matching is on the role TEXT rather than a stored name because the prompt is the
 * only thing that crosses the wire — a step carries its instruction, not a persona
 * field. Longest role first, so a role that is a prefix of another cannot shadow it.
 */
export function activePersona(prompt: string): string | null {
  const names = personaNames()
    .slice()
    .sort((a, b) => (PERSONAS[b] ?? "").length - (PERSONAS[a] ?? "").length);
  for (const name of names) {
    const role = PERSONAS[name] ?? "";
    if (role !== "" && (prompt === role || prompt.startsWith(`${role}\n\n`))) {
      return name;
    }
  }
  return null;
}

/** The prompt with its leading persona removed — the body the author wrote. */
export function promptBody(prompt: string): string {
  const active = activePersona(prompt);
  if (active === null) {
    return prompt.trim();
  }
  const role = PERSONAS[active] ?? "";
  const body = prompt === role ? "" : prompt.slice(role.length + 2);
  return body.trim();
}

/**
 * Apply `name` as the prompt's persona, or clear it with `null`.
 *
 * Always strips an existing role first, so picking twice swaps rather than stacks —
 * the one behaviour worth a test, since a stacked prompt still runs and still looks
 * plausible in the editor.
 */
export function withPersona(prompt: string, name: string | null): string {
  const body = promptBody(prompt);
  if (name === null) {
    return body;
  }
  const role = PERSONAS[name] ?? "";
  if (role === "") {
    return body;
  }
  return body ? `${role}\n\n${body}` : role;
}

/**
 * A filterable persona row that shows which persona is applied and lets it be
 * cleared.
 *
 * The filter is a plain text input rather than a `<select>`: a controlled select is
 * a known e2e hazard in this tree, and the shared capability pickers use chips for
 * the same reason.
 */
export function PersonaPicker({
  prompt,
  onChange,
  testIdBase = "step-config-persona",
}: {
  readonly prompt: string;
  readonly onChange: (nextPrompt: string) => void;
  readonly testIdBase?: string;
}) {
  const [filter, setFilter] = useState("");
  const active = activePersona(prompt);
  const needle = filter.trim().toLowerCase();
  const names = personaNames().filter((n) => n.toLowerCase().includes(needle));

  return (
    <div className="builder-chips-group">
      <input
        type="text"
        className="builder-filter"
        placeholder="Filter personas…"
        aria-label="Filter personas"
        data-testid={`${testIdBase}-filter`}
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />
      <div className="builder-chips" data-testid={testIdBase}>
        {names.map((name) => {
          const on = name === active;
          return (
            <button
              key={name}
              type="button"
              className={`chip${on ? " chip--active" : ""}`}
              aria-pressed={on}
              data-testid={`${testIdBase}-${name}`}
              // Clicking the applied persona takes it OFF. Without this the only way
              // to undo a persona was to hand-edit its text back out of the prompt.
              onClick={() => onChange(withPersona(prompt, on ? null : name))}
            >
              {name}
            </button>
          );
        })}
        {names.length === 0 ? (
          <span className="muted" data-testid={`${testIdBase}-none`}>
            No persona matches “{filter.trim()}”.
          </span>
        ) : null}
      </div>
    </div>
  );
}
