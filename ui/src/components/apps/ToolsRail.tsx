/**
 * The App MCP-TOOLS rail — attach/detach registered tools + set reach on a stored
 * App. Mirrors {@link SkillsRail}: attaching writes a WISH into the envelope
 * (`steering_config.tools.requested_grants`, mirrored to `references.tools`) and
 * re-saves via `SaveApp` — it grants NOTHING (SN-8); at RunApp the server intersects
 * the wish against the caller's grants + the live broker. A LOCKED App refuses the
 * structure edit (the control renders disabled with the reason, D142 every-state).
 * Chips, never a controlled `<select>` (the UI-3 e2e gotcha).
 */

import { toUiError } from "../../kx/errors";
import { useSaveApp } from "../../kx/use-apps";
import { useDiscoverTools } from "../../kx/use-tool-registry";
import { readReachInherit, readToolGrants, writeTools } from "../../lib/app-envelope";

export function ToolsRail({
  handle,
  envelope,
  locked,
}: {
  handle: string;
  /** The stored App's parsed envelope (the `GetApp` payload). */
  envelope: Record<string, unknown>;
  locked: boolean;
}) {
  const registry = useDiscoverTools();
  const save = useSaveApp();
  const grants = readToolGrants(envelope);
  const reach = readReachInherit(envelope);
  const attachedIds = Object.keys(grants);
  const attachedSet = new Set(attachedIds);
  const attachable = registry.tools.filter((t) => !attachedSet.has(t.toolName));
  const err = save.error ? toUiError(save.error) : null;

  const commit = (nextGrants: Record<string, string>, nextReach = reach) => {
    save.mutate({ handle, envelope: writeTools(envelope, nextGrants, nextReach) });
  };
  const attach = (name: string, version: string) => commit({ ...grants, [name]: version });
  const detach = (name: string) => {
    const { [name]: _drop, ...rest } = grants;
    commit(rest);
  };

  return (
    <div className="skills-rail" data-testid="app-tools-rail">
      <h3>MCP Tools</h3>
      <p className="muted">
        Attached tools are <em>wishes</em> — granted only at run (
        <code className="mono">wish ∩ grants ∩ fireable</code>), never by attaching (SN-8).
        {locked ? " App is locked — unlock to change tools." : ""}
      </p>
      <div className="chip-row" data-testid="app-tools-attached">
        {attachedIds.length === 0 ? (
          <span className="muted">No tools attached.</span>
        ) : (
          attachedIds.map((id) => (
            <button
              key={id}
              type="button"
              className="chip chip--active"
              disabled={locked || save.isPending}
              title={locked ? "App is locked" : `Detach ${id}`}
              onClick={() => detach(id)}
              data-testid={`app-tool-detach-${id}`}
            >
              {id} ✕
            </button>
          ))
        )}
      </div>
      {registry.notWired ? (
        <p className="muted">Tools registry not available on this gateway.</p>
      ) : attachable.length > 0 ? (
        <div className="chip-row" data-testid="app-tools-attachable">
          {attachable.map((t) => (
            <button
              key={t.toolId}
              type="button"
              className="chip"
              disabled={locked || save.isPending}
              title={
                locked ? "App is locked" : `Attach ${t.toolName}${t.kind ? ` (${t.kind})` : ""}`
              }
              onClick={() => attach(t.toolName, t.toolVersion)}
              data-testid={`app-tool-attach-${t.toolName}`}
            >
              + {t.toolName}
            </button>
          ))}
        </div>
      ) : null}
      <label className="muted" title="Let attached tools act with the caller's full reach at run">
        <input
          type="checkbox"
          checked={reach}
          disabled={locked || save.isPending}
          onChange={() => commit(grants, !reach)}
          data-testid="app-tools-reach"
        />{" "}
        Inherit principal reach
      </label>
      {err ? (
        <p className="field-error" data-testid="app-tools-error">
          {err.message}
        </p>
      ) : null}
    </div>
  );
}
