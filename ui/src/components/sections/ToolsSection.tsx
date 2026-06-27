import { useState } from "react";
import { toUiError } from "../../kx/errors";
import { useScoreBundle, useToolManifests } from "../../kx/use-toolscout";
import type { ToolsTab } from "../../router/routes/tools";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AutoGrantStatus } from "../tools/AutoGrantStatus";
import { BundleComposer } from "../tools/BundleComposer";
import { ConnectionsPanel } from "../tools/ConnectionsPanel";
import { ManifestGrid } from "../tools/ManifestGrid";
import { RegisterToolForm } from "../tools/RegisterToolForm";
import { RegisteredToolsPanel } from "../tools/RegisteredToolsPanel";
import { ScoreLadder } from "../tools/ScoreLadder";
import { SecretsPanel } from "../tools/SecretsPanel";
import { TriggersPanel } from "../tools/TriggersPanel";

const TABS: ReadonlyArray<{ id: ToolsTab; label: string }> = [
  { id: "tools", label: "Tools" },
  { id: "connections", label: "Connections" },
  { id: "triggers", label: "Triggers" },
  { id: "secrets", label: "Secrets" },
];

/**
 * Integrations — the hub over the gateway's tool + integration plane. FOUR
 * URL-addressable tabs (the ContextSection/SystemsSection view-toggle precedent —
 * tab state rides the route's validated search so this stays a pure renderer):
 *
 * 1. **Tools** — the durable tool inventory (`DiscoverTools`) + register/deregister
 *    controls, the autonomous-access posture, and the advisory toolscout (manifests
 *    + a dry-run TaskBundle scorer). Registration grants NO authority (SN-8); every
 *    score/verdict is display-only and never authorizes anything.
 * 2. **Connections** — dial external MCP servers (the live untrusted-egress surface).
 * 3. **Triggers** — bind an inbound event (webhook / cron / RPC) to a recipe handle.
 * 4. **Secrets** — the local OS-keychain store; a `SecretRef` NAME is what a
 *    Connection's / Trigger's `credential_ref` points at (the value is write-only, D81).
 *
 * Each surface degrades to an honest not-wired empty state on older gateways
 * (UNIMPLEMENTED — GR15 don't-fake-gaps).
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
          <h1>Integrations</h1>
          <p className="muted">
            Register, govern, and connect the tools, external servers, event triggers, and secrets
            your agents use. Registration grants no authority (SN-8): a tool fires only under a
            server-issued warrant, re-verified by the broker at every call.
          </p>
        </div>
      </div>

      <fieldset className="view-toggle" aria-label="Integrations view" data-testid="tools-tabs">
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

      {tab === "connections" ? (
        <ConnectionsTab />
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

/** The Connections tab — the live external-MCP-gateway govern surface. */
function ConnectionsTab() {
  return (
    <>
      <p className="muted">
        Dial external MCP servers (stdio · HTTP, including Py/TS-SDK-exposed gateways) to give your
        agents external knowledge + actions. Registering DIALS the server and registers its tools;
        secret-less credential references only (OAuth + a credential marketplace are a Cloud
        capability).
      </p>
      <ConnectionsPanel />
    </>
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
        Advisory by construction (SN-8): ranking scores and dry-run verdicts are display-only — they
        never authorize a tool.
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
