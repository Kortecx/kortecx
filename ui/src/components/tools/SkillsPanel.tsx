/**
 * The Skills panel (RC-SW1) — the govern surface over the per-principal skill
 * catalog. A skill is a DECLARATIVE `kortecx.skill/v1` bundle: instructions +
 * a tool grant-WISH set. Adding one grants NOTHING (SN-8): at run the server
 * intersects the wish against the caller's grants and the live broker
 * (`wish ∩ grants ∩ fireable`) — the wish chips here show the ADVISORY
 * `registered` bit (could THIS serve currently fire it), never authority.
 *
 * List + expand (the form: wishes + instructions preview) + add (paste the
 * manifest JSON + the instructions markdown) + remove. Identity
 * (`skillRef` / `instructionsRef`) is server-derived. Degrades to a not-wired
 * empty state on an old gateway (UNIMPLEMENTED). Every state designed (D142):
 * not-wired / loading / error / empty / populated / add-failure.
 */

import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useAddSkill, useListSkills, useRemoveSkill, useSkillForm } from "../../kx/use-skills";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { GlowCard } from "../ds/GlowCard";

const MANIFEST_PLACEHOLDER = `{
  "schema": "kortecx.skill/v1",
  "name": "my-skill",
  "version": "1",
  "description": "What outcome this skill produces.",
  "tools": { "retrieve": "1" }
}`;

/** One expanded skill row: the form (wishes + the registered bit + preview). */
function SkillDetail({ name }: { name: string }) {
  const { form, isLoading, isError, error } = useSkillForm(name);
  if (isLoading) {
    return <p className="muted">Loading skill…</p>;
  }
  if (isError) {
    return <ErrorNotice error={toUiError(error)} />;
  }
  if (!form) {
    return <p className="muted">Skill not found.</p>;
  }
  return (
    <div className="skill-detail" data-testid={`skill-detail-${name}`}>
      <p className="muted mono">instructions_ref {form.summary.instructionsRef}</p>
      {form.wishes.length === 0 ? (
        <p className="muted">No tool wishes — an instructions-only skill.</p>
      ) : (
        <div className="chip-row" data-testid={`skill-wishes-${name}`}>
          {form.wishes.map((w) => (
            <span
              key={w.toolId}
              className={`chip chip--static${w.registered ? "" : " chip--soon"}`}
              title={
                w.registered
                  ? "This serve can currently fire it (advisory — granted only at run, wish ∩ grants ∩ fireable)"
                  : "Not fireable on this serve (unregistered / undialed) — the wish will be dropped at run"
              }
            >
              {w.toolId}@{w.toolVersion}
              {w.registered ? "" : " ⨯"}
            </span>
          ))}
        </div>
      )}
      {form.instructionsPreview ? (
        <pre className="skill-detail__preview mono">
          {form.instructionsPreview}
          {form.previewTruncated ? "\n…(truncated preview)" : ""}
        </pre>
      ) : (
        <p className="muted">Instructions stored by ref (no preview).</p>
      )}
    </div>
  );
}

/** The add form: paste the manifest JSON + the instructions markdown. */
function AddSkillForm() {
  const add = useAddSkill();
  const [manifest, setManifest] = useState("");
  const [instructions, setInstructions] = useState("");
  // A LOCAL parse error surfaces the malformed-JSON state (D142: every state
  // designed) — the server never sees it, so without this the button looked dead.
  const [parseError, setParseError] = useState<string | null>(null);
  const serverErr = add.error ? toUiError(add.error) : null;
  const err = parseError ?? serverErr?.message ?? null;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(manifest) as Record<string, unknown>;
    } catch (parseEx) {
      setParseError(
        `Manifest is not valid JSON: ${parseEx instanceof Error ? parseEx.message : "parse error"}`,
      );
      return;
    }
    setParseError(null);
    add.mutate(
      { manifest: parsed, instructions: instructions || undefined },
      { onSuccess: () => setManifest("") },
    );
  };

  return (
    <form className="register-tool-form" onSubmit={onSubmit} data-testid="skill-add-form">
      <h3>Add a skill</h3>
      <p className="muted">
        Paste a <code className="mono">kortecx.skill/v1</code> manifest + the instructions markdown.
        The server validates fail-closed (authority keys refused), stores the body
        content-addressed, and derives the identity. A skill wishes; it never grants — scaffold one
        with <code className="mono">kx new skill</code>.
      </p>
      <label>
        Manifest (skill.json)
        <textarea
          required
          rows={7}
          value={manifest}
          placeholder={MANIFEST_PLACEHOLDER}
          onChange={(e) => setManifest(e.target.value)}
          data-testid="skill-add-manifest"
        />
      </label>
      <label>
        Instructions (markdown; omit iff the manifest names an instructions_ref)
        <textarea
          rows={5}
          value={instructions}
          placeholder="# My skill…"
          onChange={(e) => setInstructions(e.target.value)}
          data-testid="skill-add-instructions"
        />
      </label>
      <button
        type="submit"
        className="chip"
        disabled={add.isPending}
        data-testid="skill-add-submit"
      >
        {add.isPending ? "Adding…" : "Add skill"}
      </button>
      {err ? (
        <p className="field-error" data-testid="skill-add-error">
          {err}
        </p>
      ) : add.isSuccess ? (
        <p className="muted" data-testid="skill-add-result">
          Added {add.data.name}
          {add.data.deduplicated ? " (unchanged — identical manifest already bound)" : ""}.
        </p>
      ) : null}
    </form>
  );
}

/** The Skills catalog panel (the Integrations "Skills" tab). */
export function SkillsPanel() {
  const list = useListSkills();
  const remove = useRemoveSkill();
  const [openSkill, setOpenSkill] = useState<string | null>(null);

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="skills-panel">
      <h2>Skills</h2>
      <p className="muted">
        Declarative <code className="mono">kortecx.skill/v1</code> bundles — instructions + tool
        grant-<em>wishes</em>. Attach one to an App (CLI{" "}
        <code className="mono">kx app new … --skill</code>, SDK{" "}
        <code className="mono">.skill(…)</code>); at run the server grants only{" "}
        <code className="mono">wish ∩ grants ∩ fireable</code> — a skill on its own grants nothing.
      </p>

      {list.notWired ? (
        <EmptyState
          title="Skill catalog not available"
          detail="This gateway predates the skill catalog (RC-SW1). Upgrade the serve to govern skills here."
        />
      ) : list.isError ? (
        <ErrorNotice error={toUiError(list.error)} />
      ) : list.isLoading ? (
        <EmptyState title="Loading skills…" />
      ) : list.skills.length === 0 ? (
        <EmptyState
          title="No skills in the catalog"
          detail="Add one below, or scaffold a pack with `kx new skill <name>` and `kx skills add --dir <pack>`."
        />
      ) : (
        <ul className="connections-list" data-testid="skills-list">
          {list.skills.map((s) => {
            const open = openSkill === s.name;
            const busy = remove.isPending && remove.variables === s.name;
            return (
              <li
                key={s.skillRef}
                className="connections-list__row"
                data-testid={`skill-${s.name}`}
              >
                <div className="connections-list__head">
                  <span className="mono">
                    {s.name}@{s.version}
                  </span>
                  <span className="muted">
                    {Object.keys(s.tools).length} tool wish(es)
                    {s.description ? ` — ${s.description}` : ""}
                  </span>
                  <span className="chip-row">
                    <button
                      type="button"
                      className="chip"
                      aria-pressed={open}
                      onClick={() => setOpenSkill(open ? null : s.name)}
                      data-testid={`skill-show-${s.name}`}
                    >
                      {open ? "Hide" : "Show"}
                    </button>
                    <button
                      type="button"
                      className="chip chip--danger"
                      disabled={busy}
                      onClick={() => remove.mutate(s.name)}
                      data-testid={`skill-remove-${s.name}`}
                    >
                      {busy ? "Removing…" : "Remove"}
                    </button>
                  </span>
                </div>
                {open ? <SkillDetail name={s.name} /> : null}
              </li>
            );
          })}
        </ul>
      )}

      {list.notWired ? null : <AddSkillForm />}
    </GlowCard>
  );
}
