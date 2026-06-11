import { useState } from "react";
import { toUiError } from "../../kx/errors";
import { useScoreBundle, useToolManifests } from "../../kx/use-toolscout";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { BundleComposer } from "../tools/BundleComposer";
import { ManifestGrid } from "../tools/ManifestGrid";
import { ScoreLadder } from "../tools/ScoreLadder";

/**
 * Tools (W1.A5 toolscout): the registered tool manifests + an interactive
 * TaskBundle preview — compose an ordered tool sequence, give an intent, and
 * dry-run the advisory scorer. ADVISORY-ONLY BY CONSTRUCTION (SN-8): every
 * score/verdict here is display-only and never authorizes anything — the sole
 * grant gate stays the exact (toolId, toolVersion) check in lowering + the
 * broker. Degrades to a not-wired empty state on older gateways (UNIMPLEMENTED).
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
        MCP tool discovery &amp; TaskBundle preview. Advisory by construction (SN-8): ranking scores
        and dry-run verdicts are display-only — they never authorize a tool.
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
