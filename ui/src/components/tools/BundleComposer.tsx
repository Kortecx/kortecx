import type { ToolManifest } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { type FormEvent, useState } from "react";
import { buttonHover } from "../../app/motion";

/**
 * Compose a TaskBundle: an ORDERED tool sequence picked via CHIP buttons (the
 * recipe-chip precedent — native buttons, never a controlled `<select>`) plus a
 * free-text intent. Selected chips show their 1-based sequence position. Scoring
 * is an advisory DRY-RUN (SN-8): nothing submits, nothing journals.
 */
export function BundleComposer({
  manifests,
  selected,
  onToggle,
  pending,
  onScore,
}: {
  manifests: readonly ToolManifest[];
  selected: readonly string[];
  onToggle: (toolId: string) => void;
  pending: boolean;
  onScore: (intent: string) => void;
}) {
  const [intent, setIntent] = useState("");
  const ready = intent.trim().length > 0 && selected.length > 0;

  function submit(e: FormEvent<HTMLFormElement>): void {
    e.preventDefault();
    if (ready && !pending) {
      onScore(intent.trim());
    }
  }

  return (
    <div className="bundle-composer" data-testid="bundle-composer">
      <h2>Bundle preview</h2>
      <p className="muted">Pick a tool sequence and describe the task, then score the bundle.</p>
      {/* Toggle buttons (aria-pressed) in a plain row — biome prefers semantic
          elements over a `role="group"` div, and each chip is self-describing. */}
      <div className="chip-row">
        {manifests.map((man) => {
          const pos = selected.indexOf(man.toolId);
          const active = pos >= 0;
          return (
            <button
              key={man.toolId}
              type="button"
              data-testid={`tool-chip-${man.toolId}`}
              className={`tool-chip${active ? " tool-chip--active" : ""}`}
              aria-pressed={active}
              onClick={() => onToggle(man.toolId)}
            >
              {active ? <span className="tool-chip__pos">{pos + 1}</span> : null}
              {man.toolId}
            </button>
          );
        })}
      </div>
      <form className="bundle-form" onSubmit={submit}>
        <label htmlFor="bundle-intent">Intent</label>
        <input
          id="bundle-intent"
          data-testid="bundle-intent"
          value={intent}
          onChange={(e) => setIntent(e.target.value)}
          placeholder="What should this bundle do?"
          spellCheck={false}
          autoComplete="off"
        />
        <m.button
          type="submit"
          data-testid="bundle-score"
          disabled={!ready || pending}
          {...buttonHover}
        >
          {pending ? "Scoring…" : "Score bundle"}
        </m.button>
      </form>
    </div>
  );
}
