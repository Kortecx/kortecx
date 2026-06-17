import { useState } from "react";
import { toUiError } from "../../kx/errors";
import { useScoreBundle, useToolManifests } from "../../kx/use-toolscout";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { AutoGrantStatus } from "../tools/AutoGrantStatus";
import { BundleComposer } from "../tools/BundleComposer";
import { ConnectionsPanel } from "../tools/ConnectionsPanel";
import { ManifestGrid } from "../tools/ManifestGrid";
import { RegisterToolForm } from "../tools/RegisterToolForm";
import { RegisteredToolsPanel } from "../tools/RegisteredToolsPanel";
import { ScoreLadder } from "../tools/ScoreLadder";

/**
 * Tools — two surfaces over the gateway's tool plane:
 *
 * 1. **Registry (governance)** — the DURABLE inventory (`DiscoverTools`): every
 *    registered tool with its authority/provenance/status + register & deregister
 *    controls (`RegisterTool` / `DeregisterTool`). Registration grants NO authority
 *    (SN-8); built-ins are re-seeded + not deregisterable. Live external-MCP dialing
 *    + Connections is the PR-6b card (honest-disabled — GR19/GR15).
 * 2. **Discovery & preview (advisory)** — the W1.A5 toolscout: tool manifests + an
 *    interactive TaskBundle dry-run scorer. ADVISORY-ONLY BY CONSTRUCTION (SN-8):
 *    every score/verdict is display-only and never authorizes anything — the sole
 *    grant gate stays the exact (toolId, toolVersion) check in lowering + the broker.
 *
 * Both degrade to a not-wired empty state on older gateways (UNIMPLEMENTED).
 */
export function ToolsSection() {
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
    <section className="screen" data-testid="tools-section">
      <h1>Tools</h1>
      <p className="muted">
        Register, govern, and discover the tools your agents can call. Registration grants no
        authority (SN-8): a tool fires only under a server-issued warrant, re-verified by the broker
        at every call.
      </p>

      <h2>Registry</h2>
      <p className="muted">
        The durable tool inventory — what is registered, with what provenance, status, and egress
        authority. Built-ins are re-seeded on start and cannot be deregistered.
      </p>
      <RegisteredToolsPanel />
      <div className="tools-registry-actions">
        <RegisterToolForm />
      </div>

      <h2>Connections</h2>
      <p className="muted">
        Dial external MCP servers (stdio · HTTP, including Py/TS-SDK-exposed gateways) to give your
        agents external knowledge + actions. Registering DIALS the server and registers its tools;
        secret-less credential references only (OAuth + a credential marketplace are a Cloud
        capability).
      </p>
      <ConnectionsPanel />

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
    </section>
  );
}
