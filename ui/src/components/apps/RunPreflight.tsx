/**
 * The Run-drawer PREFLIGHT — an honest, advisory feasibility check shown BEFORE a run
 * fires, so "Run" never looks successful while silently no-op-ing. It reconciles the
 * App's needs (`GetAppManifest`: requested tools / connections / model route diffed
 * against live policy) with whether a model is served at all (`ListModels`):
 *
 *  - no model served  → agent steps pass through their prompt WITHOUT reasoning (the
 *    single biggest reliability illusion — the run completes but does not think);
 *  - the App's model route isn't served → it falls back / no-ops;
 *  - requested tools not in policy / connectors not connected → they won't fire.
 *
 * Advisory only (gates nothing — the server re-resolves at run); it just tells the
 * truth up front. Silent (renders nothing) when everything checks out on a served
 * model, or while the checks are still loading.
 */

import { useAppManifest } from "../../kx/use-app-manifest";
import { useModels } from "../../kx/use-models";

export function RunPreflight({ handle }: { handle: string }) {
  const { view, isLoading } = useAppManifest(handle);
  const { models, loading: modelsLoading } = useModels();

  if (isLoading || modelsLoading) {
    return null; // don't flash a verdict before the checks resolve
  }

  const modelKnown = models !== undefined;
  const noModel = modelKnown && models.length === 0;
  const missingTools = view
    ? view.tools.filter((t) => t.requested && !t.inPolicy && !t.inherited)
    : [];
  const missingConns = view ? view.connections.filter((c) => c.requested && !c.inPolicy) : [];
  const appModelUnserved = view ? view.modelRoute !== "" && !view.modelRouteServed : false;

  const warnings: { key: string; text: string }[] = [];
  if (noModel) {
    warnings.push({
      key: "nomodel",
      text: "⚠ No model is served — agent steps pass through their prompt without reasoning (the run completes but does not think).",
    });
  }
  if (appModelUnserved) {
    warnings.push({
      key: "model",
      text: `⚠ This App's model "${view?.modelRoute}" isn't served here — it will fall back or no-op.`,
    });
  }
  if (missingTools.length > 0) {
    warnings.push({
      key: "tools",
      text: `⚠ Tools not available (won't fire): ${missingTools.map((t) => t.id).join(", ")}.`,
    });
  }
  if (missingConns.length > 0) {
    warnings.push({
      key: "connections",
      text: `⚠ Integrations not connected: ${missingConns.map((c) => c.id).join(", ")}.`,
    });
  }

  if (warnings.length === 0) {
    // Only claim "ready" when a model is actually served (else stay silent).
    if (modelKnown && models.length > 0) {
      return (
        <p className="run-preflight run-preflight--ok" data-testid="app-run-preflight">
          <span data-testid="app-run-preflight-ready">
            ✓ Ready — a model is served and this App's capabilities are in policy.
          </span>
        </p>
      );
    }
    return null;
  }

  return (
    <div className="run-preflight run-preflight--warn" data-testid="app-run-preflight">
      <strong>Before you run</strong>
      <ul>
        {warnings.map((w) => (
          <li key={w.key} data-testid={`app-run-preflight-${w.key}`}>
            {w.text}
          </li>
        ))}
      </ul>
    </div>
  );
}
