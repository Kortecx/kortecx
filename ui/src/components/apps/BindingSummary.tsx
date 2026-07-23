/**
 * The "binds to" summary shown under an App detail rail.
 *
 * A rail edits `references.*` — the DECLARATION (what the App has registered). But a
 * capability binds to the NODE that names it, so a rail that showed an attached skill with
 * no hint of where it acts would read as app-wide when the truth is per-node — the exact
 * dishonesty the per-node model exists to remove. This states the runtime's own rule on
 * screen: each declared name, and the step(s) it binds to (or "the entry step" when no step
 * claims it). Re-binding happens on the canvas (Lineage → Edit structure); this is read-only.
 */

import { bindingTargets } from "../../lib/app-envelope";

export function BindingSummary({
  envelope,
  axis,
  names,
  label,
  testId,
}: {
  envelope: Record<string, unknown>;
  axis: "skills" | "connections" | "datasets";
  /** The declared names to explain (a rail's own `references.*` entries — for connections,
   *  the raw endpoint descriptor the blueprint step binds by). */
  names: readonly string[];
  /** Optional friendly display for a name (a connector endpoint is not what a person
   *  recognises); binding still MATCHES on the raw name. */
  label?: (name: string) => string;
  testId: string;
}) {
  if (names.length === 0) {
    return null;
  }
  return (
    <ul className="app-binding-summary muted" data-testid={testId}>
      {names.map((name) => {
        const targets = bindingTargets(envelope, axis, name);
        return (
          <li key={name} data-testid={`${testId}-${name}`}>
            <strong>{label ? label(name) : name}</strong> binds to {targets.join(", ")}
          </li>
        );
      })}
    </ul>
  );
}
