/**
 * The App SKILLS rail — attach/detach catalog skills on a stored App.
 *
 * A skill is a DECLARATIVE `kortecx.skill/v1` bundle the envelope references by
 * `SkillRef { name, instructions_ref, tools }` (`references.skills`). Attaching
 * grants NOTHING (SN-8): at `RunApp` the server intersects the skill's tool
 * WISHES against the caller's grants and the live broker. Attach/detach is a
 * structure edit — it re-saves the envelope (`SaveApp`), so a LOCKED App
 * refuses it (the POC-5d lock covers structure edits; the control renders
 * disabled with the reason, D142 every-state).
 *
 * The chips themselves are {@link SkillsPicker} — the SAME picker the New App create
 * form mounts, so "attach at create" and "attach after create" cannot drift. This rail
 * is now just the envelope adapter: read `references.skills`, write it back through
 * `SaveApp`.
 */

import { toUiError } from "../../kx/errors";
import { useSaveApp } from "../../kx/use-apps";
import { unbindFromSteps } from "../../lib/app-envelope";
import { BindingSummary } from "./BindingSummary";
import { type PickedSkill, SkillsPicker } from "./CapabilityPickers";

interface SkillRefJson {
  name: string;
  instructions_ref: string;
  tools?: Record<string, string>;
}

/** The envelope's attached skills (tolerant read of the opaque JSON). */
function attachedSkills(envelope: Record<string, unknown>): SkillRefJson[] {
  const refs = envelope.references as { skills?: SkillRefJson[] } | undefined;
  return refs?.skills ?? [];
}

export function SkillsRail({
  handle,
  envelope,
  locked,
}: {
  handle: string;
  /** The stored App's parsed envelope (the `GetApp` payload). */
  envelope: Record<string, unknown>;
  locked: boolean;
}) {
  const save = useSaveApp();
  const attached = attachedSkills(envelope);
  const err = save.error ? toUiError(save.error) : null;

  // The ONE place the authoring vocabulary (`instructionsRef`, what the SDK `.skill()`
  // takes) meets the envelope's wire spelling (`instructions_ref`). The picker speaks
  // the former on both surfaces; only this rail writes the envelope.
  const picked: PickedSkill[] = attached.map((s) => ({
    name: s.name,
    instructionsRef: s.instructions_ref,
    ...(s.tools && Object.keys(s.tools).length > 0 ? { tools: s.tools } : {}),
  }));

  const commit = (next: PickedSkill[]) => {
    const skills: SkillRefJson[] = next.map((s) => ({
      name: s.name,
      instructions_ref: s.instructionsRef,
      ...(s.tools && Object.keys(s.tools).length > 0 ? { tools: s.tools } : {}),
    }));
    // Detaching a skill scrubs its binding from every step too, so the blueprint never names
    // a step-binding to a `references.skills` entry that is gone.
    const kept = new Set(next.map((s) => s.name));
    let base = envelope;
    for (const s of attached) {
      if (!kept.has(s.name)) {
        base = unbindFromSteps(base, "skills", s.name);
      }
    }
    // Omit-empty without `delete` (biome perf rule): rebuild the objects.
    const { skills: _drop, ...restRefs } = {
      ...(base.references as Record<string, unknown> | undefined),
    };
    const refs: Record<string, unknown> = skills.length > 0 ? { ...restRefs, skills } : restRefs;
    const { references: _dropRefs, ...restEnv } = base;
    const nextEnv: Record<string, unknown> =
      Object.keys(refs).length > 0 ? { ...restEnv, references: refs } : restEnv;
    save.mutate({ handle, envelope: nextEnv });
  };

  return (
    <div className="skills-rail" data-testid="app-skills-rail">
      <h3>Skills</h3>
      <p className="muted">
        Attached skills are available to the App (instructions + tool <em>wishes</em>; granted only
        at run, <code className="mono">wish ∩ grants ∩ fireable</code>). Each binds to the step(s)
        that use it — edit which on the canvas (Lineage → Edit structure).
        {locked ? " App is locked — unlock to change skills." : ""}
      </p>
      <SkillsPicker
        skills={picked}
        onChange={commit}
        disabled={locked || save.isPending}
        disabledTitle={locked ? "App is locked" : "Saving…"}
        groupTestId="app-skills"
        itemTestId="app-skill"
      />
      <BindingSummary
        envelope={envelope}
        axis="skills"
        names={attached.map((s) => s.name)}
        testId="app-skills-binds"
      />
      {err ? (
        <p className="field-error" data-testid="app-skills-error">
          {err.message}
        </p>
      ) : null}
    </div>
  );
}
