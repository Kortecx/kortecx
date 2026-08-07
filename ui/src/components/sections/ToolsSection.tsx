import { useState } from "react";
import { toUiError } from "../../kx/errors";
import { useScoreBundle, useToolManifests } from "../../kx/use-toolscout";
import type { ToolsTab } from "../../router/routes/tools";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AutoGrantStatus } from "../tools/AutoGrantStatus";
import { BundleComposer } from "../tools/BundleComposer";
import { ConnectorsPanel } from "../tools/ConnectorsPanel";
import { ManifestGrid } from "../tools/ManifestGrid";
import { RegisterToolForm } from "../tools/RegisterToolForm";
import { RegisteredToolsPanel } from "../tools/RegisteredToolsPanel";
import { ScoreLadder } from "../tools/ScoreLadder";
import { ScriptsPanel } from "../tools/ScriptsPanel";
import { SecretsPanel } from "../tools/SecretsPanel";
import { SkillsPanel } from "../tools/SkillsPanel";
import { TriggersPanel } from "../tools/TriggersPanel";

const TABS: ReadonlyArray<{ id: ToolsTab; label: string }> = [
  { id: "tools", label: "Tools" },
  { id: "scripts", label: "Scripts" },
  { id: "connectors", label: "Connectors" },
  { id: "skills", label: "Skills" },
  { id: "triggers", label: "Triggers" },
  { id: "secrets", label: "Secrets" },
];

/**
 * MCP — the hub over everything an agent can call. URL-addressable tabs (the
 * ContextSection/SystemsSection view-toggle precedent — tab state rides the
 * route's validated search so this stays a pure renderer):
 *
 * 1. **Tools** — the durable tool inventory (`DiscoverTools`) + register/deregister
 *    controls, the autonomous-access posture, and the advisory toolscout (manifests
 *    + a dry-run TaskBundle scorer). Registration grants NO authority; every
 *    score/verdict is display-only and never authorizes anything.
 * 2. **Scripts** — the durable script registry (`ListScripts`) + register/deregister.
 *    A script is a tool whose declaration carries source, an interpreter and a
 *    resource wish; it runs in the platform sandbox under the CALLER's grants, and
 *    a serve that cannot sandbox refuses to register one at all.
 * 3. **Connectors** — everything the runtime can reach outside itself: the connectors that
 *    ship in the box and any other MCP server you connect, in ONE list. A row says whether
 *    it is set up, and acting on the row sets it up. Catalog and instance are two states of
 *    a row, not two destinations.
 * 4. **Skills** — the declarative skill artifacts an agent can apply.
 * 5. **Triggers** — bind an inbound event (webhook / cron / RPC) to a recipe handle.
 * 6. **Secrets** — the local secret store; a `SecretRef` NAME is what a connector's or
 *    trigger's credential reference points at (the value is write-only, D81).
 *
 * Each surface degrades to an honest not-wired empty state on older gateways
 * (UNIMPLEMENTED — don't-fake-gaps).
 */
export function ToolsSection({
  tab = "tools",
  onTab,
}: {
  tab?: ToolsTab;
  onTab?: (tab: ToolsTab) => void;
} = {}) {
  return (
    <section className="screen" data-testid="tools-section">
      <div className="section-head">
        <div>
          <h1>MCP</h1>
          <p className="muted">
            Register, govern, and connect everything your agents can call — tools, sandboxed
            scripts, connectors, skills, event triggers and secrets. Registration grants no
            authority: anything here fires only under a server-issued warrant, re-verified at every
            call.
          </p>
        </div>
      </div>

      <fieldset className="view-toggle" aria-label="MCP view" data-testid="tools-tabs">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            data-testid={`tools-tab-${t.id}`}
            aria-pressed={tab === t.id}
            onClick={() => onTab?.(t.id)}
          >
            {t.label}
          </button>
        ))}
      </fieldset>

      {tab === "scripts" ? (
        <ScriptsPanel />
      ) : tab === "connectors" ? (
        <ConnectorsPanel />
      ) : tab === "skills" ? (
        <SkillsPanel />
      ) : tab === "triggers" ? (
        <TriggersPanel />
      ) : tab === "secrets" ? (
        <SecretsPanel />
      ) : (
        <ToolsTabBody />
      )}
    </section>
  );
}

/** The Tools tab — registry, autonomous-access posture, and the advisory toolscout. */
function ToolsTabBody() {
  const manifests = useToolManifests();
  const score = useScoreBundle();
  const [selected, setSelected] = useState<readonly string[]>([]);

  const list = manifests.data ?? [];
  const notWired = manifests.isError && toUiError(manifests.error).kind === "not-wired";

  function toggle(toolId: string): void {
    setSelected((prev) =>
      prev.includes(toolId) ? prev.filter((id) => id !== toolId) : [...prev, toolId],
    );
  }

  function runScore(intent: string): void {
    const tools = selected.flatMap((id) => {
      const man = list.find((candidate) => candidate.toolId === id);
      return man ? [{ toolId: man.toolId, toolVersion: man.toolVersion }] : [];
    });
    score.mutate({ intent, languageTags: ["en"], tools });
  }

  const scoreError = score.error ? toUiError(score.error) : null;

  return (
    <>
      <h2>Registry</h2>
      <p className="muted">
        The durable tool inventory — what is registered, with what provenance, status, and egress
        authority. Built-ins are re-seeded on start and cannot be deregistered.
      </p>
      <RegisteredToolsPanel />
      <div className="tools-registry-actions">
        <RegisterToolForm />
      </div>

      <h2>Autonomous tool access</h2>
      <p className="muted">
        Whether the autonomous agent loop may auto-grant the registered and dialed tool set. The
        runtime is the source of truth — OSS exposes no toggle here; the operator enables it at
        startup (<span className="mono">KX_SERVE_AUTOGRANT</span>).
      </p>
      <AutoGrantStatus />

      <h2>Discovery &amp; preview</h2>
      <p className="muted">
        Advisory by construction: ranking scores and dry-run verdicts are display-only — they never
        authorize a tool.
      </p>

      {manifests.isLoading ? <EmptyState title="Loading tools…" /> : null}
      {notWired ? (
        <EmptyState
          title="Tool discovery needs a newer gateway"
          detail="This gateway does not expose the toolscout viewer (an older build)."
        />
      ) : null}
      {manifests.isError && !notWired ? (
        <EmptyState title="Couldn't load tools" detail={toUiError(manifests.error).message} />
      ) : null}
      {manifests.data && list.length === 0 ? (
        <EmptyState
          title="No tools registered"
          detail="This gateway registers no tool manifests."
        />
      ) : null}

      {list.length > 0 ? (
        <>
          <ManifestGrid manifests={list} selected={selected} onToggle={toggle} />
          <BundleComposer
            manifests={list}
            selected={selected}
            onToggle={toggle}
            pending={score.isPending}
            onScore={runScore}
          />
          {scoreError ? <ErrorNotice error={scoreError} onRetry={() => score.reset()} /> : null}
          {score.data ? <ScoreLadder score={score.data} /> : null}
        </>
      ) : null}
    </>
  );
}
