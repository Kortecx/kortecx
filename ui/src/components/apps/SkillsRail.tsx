/**
 * The App SKILLS rail — attach/detach catalog skills on a stored App.
 *
 * A skill is a DECLARATIVE `kortecx.skill/v1` bundle the envelope references by
 * `SkillRef { name, instructions_ref, tools }` (`references.skills`). Attaching
 * grants NOTHING (SN-8): at `RunApp` the server intersects the skill's tool
 * WISHES against the caller's grants and the live broker. Attach/detach is a
 * structure edit — it re-saves the envelope (`SaveApp`), so a LOCKED App
 * refuses it (the POC-5d lock covers structure edits; the control renders
 * disabled with the reason, D142 every-state). Chips, never a controlled
 * `<select>` (the UI-3 e2e gotcha).
 */

import { toUiError } from "../../kx/errors";
import { useSaveApp } from "../../kx/use-apps";
import { useListSkills } from "../../kx/use-skills";

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
  const catalog = useListSkills();
  const save = useSaveApp();
  const attached = attachedSkills(envelope);
  const attachedNames = new Set(attached.map((s) => s.name));
  const attachable = catalog.skills.filter((s) => !attachedNames.has(s.name));
  const err = save.error ? toUiError(save.error) : null;

  const mutate = (skills: SkillRefJson[]) => {
    // Omit-empty without `delete` (biome perf rule): rebuild the objects.
    const { skills: _drop, ...restRefs } = {
      ...(envelope.references as Record<string, unknown> | undefined),
    };
    const refs: Record<string, unknown> = skills.length > 0 ? { ...restRefs, skills } : restRefs;
    const { references: _dropRefs, ...restEnv } = envelope;
    const next: Record<string, unknown> =
      Object.keys(refs).length > 0 ? { ...restEnv, references: refs } : restEnv;
    save.mutate({ handle, envelope: next });
  };

  const attach = (name: string) => {
    const s = catalog.skills.find((c) => c.name === name);
    if (!s) {
      return;
    }
    mutate([
      ...attached,
      {
        name: s.name,
        instructions_ref: s.instructionsRef,
        ...(Object.keys(s.tools).length > 0 ? { tools: s.tools } : {}),
      },
    ]);
  };

  const detach = (name: string) => {
    mutate(attached.filter((s) => s.name !== name));
  };

  return (
    <div className="skills-rail" data-testid="app-skills-rail">
      <h3>Skills</h3>
      <p className="muted">
        Attached skills steer the App's entry step (instructions + tool <em>wishes</em>; granted
        only at run, <code className="mono">wish ∩ grants ∩ fireable</code>).
        {locked ? " App is locked — unlock to change skills." : ""}
      </p>
      <div className="chip-row" data-testid="app-skills-attached">
        {attached.length === 0 ? (
          <span className="muted">No skills attached.</span>
        ) : (
          attached.map((s) => (
            <button
              key={s.name}
              type="button"
              className="chip chip--active"
              disabled={locked || save.isPending}
              title={locked ? "App is locked" : `Detach ${s.name}`}
              onClick={() => detach(s.name)}
              data-testid={`app-skill-detach-${s.name}`}
            >
              {s.name} ✕
            </button>
          ))
        )}
      </div>
      {catalog.notWired ? (
        <p className="muted">Skill catalog not available on this gateway.</p>
      ) : attachable.length > 0 ? (
        <div className="chip-row" data-testid="app-skills-attachable">
          {attachable.map((s) => (
            <button
              key={s.name}
              type="button"
              className="chip"
              disabled={locked || save.isPending}
              title={
                locked
                  ? "App is locked"
                  : `Attach ${s.name} (${Object.keys(s.tools).length} tool wish(es))`
              }
              onClick={() => attach(s.name)}
              data-testid={`app-skill-attach-${s.name}`}
            >
              + {s.name}
            </button>
          ))}
        </div>
      ) : null}
      {err ? (
        <p className="field-error" data-testid="app-skills-error">
          {err.message}
        </p>
      ) : null}
    </div>
  );
}
