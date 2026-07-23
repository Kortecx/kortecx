/**
 * The App MCP-TOOLS rail — attach/detach registered tools + set reach on a stored
 * App. Mirrors {@link SkillsRail}: attaching writes a WISH into the envelope
 * (`steering_config.tools.requested_grants`, mirrored to `references.tools`) and
 * re-saves via `SaveApp` — it grants NOTHING (SN-8); at RunApp the server intersects
 * the wish against the caller's grants + the live broker. A LOCKED App refuses the
 * structure edit (the control renders disabled with the reason, D142 every-state).
 *
 * The chips + the reach checkbox are {@link ToolsPicker} — the SAME picker the New App
 * create form mounts, so the two authoring surfaces cannot drift. This rail is now just
 * the envelope adapter over `writeTools`.
 */

import { toUiError } from "../../kx/errors";
import { useSaveApp } from "../../kx/use-apps";
import { readReachInherit, readToolGrants, writeTools } from "../../lib/app-envelope";
import { ToolsPicker } from "./CapabilityPickers";

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
  const save = useSaveApp();
  const grants = readToolGrants(envelope);
  const reach = readReachInherit(envelope);
  const err = save.error ? toUiError(save.error) : null;

  const commit = (nextGrants: Record<string, string>, nextReach: boolean) => {
    save.mutate({ handle, envelope: writeTools(envelope, nextGrants, nextReach) });
  };

  return (
    <div className="skills-rail" data-testid="app-tools-rail">
      <h3>MCP Tools</h3>
      <p className="muted">
        App-wide tool <em>wishes</em> — granted only at run (
        <code className="mono">wish ∩ grants ∩ fireable</code>), never by attaching (SN-8), and
        bound to the entry step. To grant a tool to a specific step, open it on the canvas (Lineage
        → Edit structure).
        {locked ? " App is locked — unlock to change tools." : ""}
      </p>
      <ToolsPicker
        grants={grants}
        reachInherit={reach}
        onChange={commit}
        disabled={locked || save.isPending}
        disabledTitle={locked ? "App is locked" : "Saving…"}
        groupTestId="app-tools"
        itemTestId="app-tool"
        reachTestId="app-tools-reach"
      />
      {err ? (
        <p className="field-error" data-testid="app-tools-error">
          {err.message}
        </p>
      ) : null}
    </div>
  );
}
