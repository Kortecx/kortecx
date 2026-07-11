import { type ManifestRow, useAppManifest } from "../../kx/use-app-manifest";

/** The status chip class + label for one tool row ("satisfied" / "missing" / "inherited"). */
function toolStatus(row: ManifestRow, needsOnly: boolean): { cls: string; label: string } {
  if (needsOnly) return { cls: "chip chip--static", label: "needs" };
  if (!row.requested && row.inherited)
    return { cls: "chip chip--static chip--tag", label: "inherited" };
  if (row.inPolicy) return { cls: "chip chip--static chip--active", label: "satisfied" };
  return { cls: "chip chip--static chip--danger", label: "missing" };
}

/**
 * The read-only capability manifest for one App — its resolved model route, tool
 * reach, and the capability ceiling (each requested tool / connection diffed against
 * your live policy, via `GetAppManifest`). Degrades to the declared-needs-only view
 * on an older gateway. Shared by the Apps "View" popover and the Security section.
 */
export function AppManifestPanel({ handle }: { handle: string }) {
  const { view, isLoading } = useAppManifest(handle);
  return (
    <>
      <h3 className="node-drawer__section">Capability manifest</h3>
      {isLoading ? <p className="muted">Resolving…</p> : null}
      {!view && !isLoading ? (
        <p className="muted" data-testid="app-manifest-empty">
          No capability manifest.
        </p>
      ) : null}
      {view ? (
        <div data-testid="app-manifest">
          {view.needsOnly ? (
            <p className="muted">Server can’t resolve your policy — showing declared needs only.</p>
          ) : null}
          <dl className="facts">
            <dt>Model</dt>
            <dd>
              {view.modelRoute === "" ? "(served default)" : view.modelRoute}
              {view.needsOnly ? null : (
                <span
                  className={`chip chip--static chip--tag ${
                    view.modelRouteServed ? "chip--active" : "chip--danger"
                  }`}
                  data-testid="app-manifest-model-status"
                >
                  {view.modelRouteServed ? "served" : "not served"}
                </span>
              )}
            </dd>
            <dt>Tool reach</dt>
            <dd>{view.reachInherit ? "inherit principal" : "explicit"}</dd>
          </dl>
          {view.tools.length > 0 ? (
            <div className="chip-row" data-testid="app-manifest-tools">
              {view.tools.map((t) => {
                const s = toolStatus(t, view.needsOnly);
                return (
                  <span key={`${t.id}@${t.version}`} className={s.cls} title={s.label}>
                    <code className="mono">
                      {t.id}@{t.version}
                    </code>
                    <small>{s.label}</small>
                  </span>
                );
              })}
            </div>
          ) : null}
          {view.connections.length > 0 ? (
            <div className="chip-row" data-testid="app-manifest-connections">
              {view.connections.map((c) => (
                <span
                  key={c.id}
                  className={`chip chip--static ${
                    view.needsOnly ? "" : c.inPolicy ? "chip--active" : "chip--danger"
                  }`}
                  title={view.needsOnly ? "needs" : c.inPolicy ? "registered" : "missing"}
                >
                  <code className="mono">{c.id}</code>
                  <small>{view.needsOnly ? "needs" : c.inPolicy ? "registered" : "missing"}</small>
                </span>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </>
  );
}
