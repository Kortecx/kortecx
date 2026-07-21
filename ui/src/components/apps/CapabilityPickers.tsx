/**
 * The shared capability PICKERS — ONE implementation of "attach a skill / a tool / an
 * integration", used by both surfaces that author an App's capabilities: the detail
 * page's editable rails ({@link SkillsRail} / {@link ToolsRail} / {@link ConnectionsRail},
 * which commit each change through `SaveApp`) and the New App create form (which folds the
 * selection into the SDK `app()` builder before the FIRST save).
 *
 * The create form had ZERO capability call sites, so a console-authored App shipped with
 * `references.tools == []`, `references.skills == []` and no connection — it ran with
 * nothing plugged in. The tempting fix (paste the rail bodies into the form) would have
 * left two copies of attach/detach free to drift, and drift here is invisible: both halves
 * type-check and only the ENVELOPE is wrong. Hence controlled pickers that own the catalog
 * query and the chip chrome and NOTHING else — who persists the value is the caller's
 * business (envelope + `SaveApp` on the detail page; local state + `.useTool()` /
 * `.skill()` / `.withConnection()` / `.steer({ reach })` on the create form).
 *
 * Attaching grants NOTHING (SN-8): every entry is a WISH the server re-intersects at
 * `RunApp` against the caller's grants and the live broker. Chips, never a controlled
 * `<select>` (the UI-3 e2e gotcha). Each catalog hook's `notWired` degradation is a
 * sentence, never an empty picker — an empty picker reads as "you have no tools"
 * (don't-fake-gaps). Test ids are caller-supplied so the shipped rail ids survive the
 * extraction unchanged.
 */

import type { Skill } from "@kortecx/sdk/web";
import { useState } from "react";
import { useListMcpServers } from "../../kx/use-connections";
import { useListSkills } from "../../kx/use-skills";
import { useDiscoverTools } from "../../kx/use-tool-registry";
import type { ConnectionEntry } from "../../lib/app-envelope";

/** One chip in a picker. `value` is both the React key and what the callbacks echo back;
 *  `label` is the face (it may differ — an integration binds by ENDPOINT but reads better
 *  as its registered server name). */
export interface CapabilityChip {
  readonly value: string;
  readonly label: string;
  readonly title: string;
}

/** The chip chrome shared by all three pickers: the attached row (click ⇒ detach), then
 *  the catalog remainder (click ⇒ attach) — or the not-wired sentence in its place. */
function CapabilityChips({
  attached,
  attachable,
  emptyNote,
  notWiredNote,
  attachedTestId,
  attachableTestId,
  attachTestId,
  detachTestId,
  disabled,
  disabledTitle,
  onAttach,
  onDetach,
}: {
  readonly attached: readonly CapabilityChip[];
  /** The catalog remainder, or `null` when the catalog is NOT WIRED on this gateway. */
  readonly attachable: readonly CapabilityChip[] | null;
  readonly emptyNote: string;
  readonly notWiredNote: string;
  readonly attachedTestId: string;
  readonly attachableTestId: string;
  readonly attachTestId: (value: string) => string;
  readonly detachTestId: (value: string) => string;
  readonly disabled: boolean;
  /** Replaces every chip title while disabled (e.g. "App is locked"). */
  readonly disabledTitle: string;
  readonly onAttach: (value: string) => void;
  readonly onDetach: (value: string) => void;
}) {
  return (
    <>
      <div className="chip-row" data-testid={attachedTestId}>
        {attached.length === 0 ? (
          <span className="muted">{emptyNote}</span>
        ) : (
          attached.map((c) => (
            <button
              key={c.value}
              type="button"
              className="chip chip--active"
              disabled={disabled}
              title={disabled ? disabledTitle : c.title}
              onClick={() => onDetach(c.value)}
              data-testid={detachTestId(c.value)}
            >
              {c.label} ✕
            </button>
          ))
        )}
      </div>
      {attachable === null ? (
        <p className="muted">{notWiredNote}</p>
      ) : attachable.length > 0 ? (
        <div className="chip-row" data-testid={attachableTestId}>
          {attachable.map((c) => (
            <button
              key={c.value}
              type="button"
              className="chip"
              disabled={disabled}
              title={disabled ? disabledTitle : c.title}
              onClick={() => onAttach(c.value)}
              data-testid={attachTestId(c.value)}
            >
              + {c.label}
            </button>
          ))}
        </div>
      ) : null}
    </>
  );
}

/**
 * A skill captured at pick time — exactly what `references.skills` / the SDK `.skill()`
 * need. The catalog row is snapshotted on ATTACH rather than re-read at save, so the save
 * path never depends on a query that may have refetched (or emptied) in between.
 *
 * Camel-cased because that is the AUTHORING vocabulary (the SDK `Skill`); the envelope's
 * `instructions_ref` mapping lives in exactly one place, {@link SkillsRail}.
 */
export interface PickedSkill extends Skill {
  readonly instructionsRef: string;
}

/** The catalog-skill picker (`references.skills`). Attached skills steer the App's entry
 *  step with their instructions + tool WISHES. */
export function SkillsPicker({
  skills,
  onChange,
  disabled = false,
  disabledTitle = "",
  groupTestId,
  itemTestId,
}: {
  readonly skills: readonly PickedSkill[];
  readonly onChange: (next: PickedSkill[]) => void;
  readonly disabled?: boolean;
  readonly disabledTitle?: string;
  /** Base for the row containers (`<group>-attached` / `<group>-attachable`). */
  readonly groupTestId: string;
  /** Base for the per-chip ids (`<item>-attach-<name>` / `<item>-detach-<name>`). */
  readonly itemTestId: string;
}) {
  const catalog = useListSkills();
  const attachedNames = new Set(skills.map((s) => s.name));

  return (
    <CapabilityChips
      attached={skills.map((s) => ({
        value: s.name,
        label: s.name,
        title: `Detach ${s.name}`,
      }))}
      attachable={
        catalog.notWired
          ? null
          : catalog.skills
              .filter((s) => !attachedNames.has(s.name))
              .map((s) => ({
                value: s.name,
                label: s.name,
                title: `Attach ${s.name} (${Object.keys(s.tools).length} tool wish(es))`,
              }))
      }
      emptyNote="No skills attached."
      notWiredNote="Skill catalog not available on this gateway."
      attachedTestId={`${groupTestId}-attached`}
      attachableTestId={`${groupTestId}-attachable`}
      attachTestId={(v) => `${itemTestId}-attach-${v}`}
      detachTestId={(v) => `${itemTestId}-detach-${v}`}
      disabled={disabled}
      disabledTitle={disabledTitle}
      onAttach={(name) => {
        const s = catalog.skills.find((c) => c.name === name);
        if (!s) {
          return;
        }
        onChange([
          ...skills,
          {
            name: s.name,
            instructionsRef: s.instructionsRef,
            ...(Object.keys(s.tools).length > 0 ? { tools: { ...s.tools } } : {}),
          },
        ]);
      }}
      onDetach={(name) => onChange(skills.filter((s) => s.name !== name))}
    />
  );
}

/** The registered-tool picker (the tool wish `id → version`) + the reach knob. Reach rides
 *  HERE rather than on the rail because it selects how the wish is resolved — a tool
 *  decision, and one the create form must be able to make too (an App authored with
 *  `inherit_principal` after the fact is a different App). */
export function ToolsPicker({
  grants,
  reachInherit,
  onChange,
  disabled = false,
  disabledTitle = "",
  groupTestId,
  itemTestId,
  reachTestId,
}: {
  readonly grants: Record<string, string>;
  readonly reachInherit: boolean;
  readonly onChange: (grants: Record<string, string>, reachInherit: boolean) => void;
  readonly disabled?: boolean;
  readonly disabledTitle?: string;
  readonly groupTestId: string;
  readonly itemTestId: string;
  readonly reachTestId: string;
}) {
  const registry = useDiscoverTools();
  const attachedIds = Object.keys(grants);
  const attachedSet = new Set(attachedIds);

  return (
    <>
      <CapabilityChips
        attached={attachedIds.map((id) => ({ value: id, label: id, title: `Detach ${id}` }))}
        attachable={
          registry.notWired
            ? null
            : registry.tools
                .filter((t) => !attachedSet.has(t.toolName))
                .map((t) => ({
                  // The wish is keyed by tool NAME; the version rides along on attach.
                  value: t.toolName,
                  label: t.toolName,
                  title: `Attach ${t.toolName}${t.kind ? ` (${t.kind})` : ""}`,
                }))
        }
        emptyNote="No tools attached."
        notWiredNote="Tools registry not available on this gateway."
        attachedTestId={`${groupTestId}-attached`}
        attachableTestId={`${groupTestId}-attachable`}
        attachTestId={(v) => `${itemTestId}-attach-${v}`}
        detachTestId={(v) => `${itemTestId}-detach-${v}`}
        disabled={disabled}
        disabledTitle={disabledTitle}
        onAttach={(name) => {
          const t = registry.tools.find((c) => c.toolName === name);
          if (!t) {
            return;
          }
          onChange({ ...grants, [name]: t.toolVersion }, reachInherit);
        }}
        onDetach={(name) => {
          const { [name]: _drop, ...rest } = grants;
          onChange(rest, reachInherit);
        }}
      />
      <label className="muted" title="Let attached tools act with the caller's full reach at run">
        <input
          type="checkbox"
          checked={reachInherit}
          disabled={disabled}
          onChange={() => onChange(grants, !reachInherit)}
          data-testid={reachTestId}
        />{" "}
        Inherit principal reach
      </label>
    </>
  );
}

/**
 * The connector picker (`references.connections`). Registered MCP servers offer a
 * one-click bind (their endpoint IS the descriptor); the manual endpoint + credential-NAME
 * row covers anything not yet registered. `ListMcpServers` returns only whether a
 * credential is present (a boolean, D81) — never the name — so a one-click bind lands
 * credential-less and the NAME is always typed.
 */
export function ConnectionsPicker({
  connections,
  onChange,
  disabled = false,
  disabledTitle = "",
  groupTestId,
  itemTestId,
}: {
  readonly connections: readonly ConnectionEntry[];
  readonly onChange: (next: ConnectionEntry[]) => void;
  readonly disabled?: boolean;
  readonly disabledTitle?: string;
  readonly groupTestId: string;
  readonly itemTestId: string;
}) {
  const registry = useListMcpServers();
  const [descriptor, setDescriptor] = useState("");
  const [credential, setCredential] = useState("");
  const bound = new Set(connections.map((c) => c.descriptor));

  const bind = (desc: string, cred: string) => {
    const d = desc.trim();
    if (d === "" || bound.has(d)) {
      return;
    }
    onChange([...connections, { descriptor: d, credential_ref: cred.trim() }]);
    setDescriptor("");
    setCredential("");
  };

  return (
    <>
      <CapabilityChips
        attached={connections.map((c) => ({
          value: c.descriptor,
          label: c.credential_ref ? `${c.descriptor} · ${c.credential_ref}` : c.descriptor,
          title: `Unbind ${c.descriptor}`,
        }))}
        attachable={
          registry.notWired
            ? null
            : registry.servers
                .filter((s) => !bound.has(s.endpoint))
                .map((s) => ({
                  // Bind by ENDPOINT (the descriptor the runtime dials); the chip reads as
                  // the registered server name, which is what the operator recognizes.
                  value: s.endpoint,
                  label: s.serverName,
                  title: `Bind ${s.serverName} (${s.endpoint})`,
                }))
        }
        emptyNote="No integrations bound."
        notWiredNote="Connector registry not available on this gateway."
        attachedTestId={`${groupTestId}-attached`}
        attachableTestId={`${groupTestId}-attachable`}
        attachTestId={(v) => `${itemTestId}-attach-${v}`}
        detachTestId={(v) => `${itemTestId}-detach-${v}`}
        disabled={disabled}
        disabledTitle={disabledTitle}
        onAttach={(endpoint) => bind(endpoint, "")}
        onDetach={(desc) => onChange(connections.filter((c) => c.descriptor !== desc))}
      />
      <div className="chip-row">
        <input
          className="input"
          placeholder="endpoint (stdio command or https URL)"
          value={descriptor}
          disabled={disabled}
          onChange={(e) => setDescriptor(e.target.value)}
          data-testid={`${itemTestId}-descriptor`}
          aria-label="Connector endpoint"
          spellCheck={false}
          autoComplete="off"
        />
        <input
          className="input"
          placeholder="credential name (optional)"
          value={credential}
          disabled={disabled}
          onChange={(e) => setCredential(e.target.value)}
          data-testid={`${itemTestId}-credential`}
          aria-label="Credential name"
          spellCheck={false}
          autoComplete="off"
        />
        <button
          type="button"
          className="btn-ghost"
          disabled={disabled || descriptor.trim() === ""}
          onClick={() => bind(descriptor, credential)}
          data-testid={`${itemTestId}-bind`}
        >
          Bind
        </button>
      </div>
    </>
  );
}
