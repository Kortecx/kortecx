import { useEffect } from "react";
import { toUiError } from "../../kx/errors";
import { useAssetGrants } from "../../kx/use-grants";
import { useRecipes } from "../../kx/use-recipes";
import { formatActions, grantStatusLabel } from "../../lib/grant-format";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";

/**
 * The sharing (grants) inspector: a CHIP picker over the inspectable assets (the
 * provisioned recipes), driving the grant table for the selected asset. Each grant is
 * fold-classified root/delegated + active/revoked. The selected asset is lifted up so
 * the teams panel can resolve each member's warrant on it.
 */
export function GrantInspector({
  selectedAsset,
  onSelectAsset,
}: {
  selectedAsset: string | null;
  onSelectAsset: (assetRef: string) => void;
}) {
  const recipes = useRecipes();
  const assets = recipes.data ?? [];
  const effective =
    selectedAsset && assets.includes(selectedAsset) ? selectedAsset : (assets[0] ?? null);

  useEffect(() => {
    const first = assets[0];
    if (!selectedAsset && first) {
      onSelectAsset(first);
    }
  }, [selectedAsset, assets, onSelectAsset]);

  const grants = useAssetGrants(effective ?? undefined);
  const catalogNotWired = recipes.isError && toUiError(recipes.error).kind === "not-wired";

  return (
    <div data-testid="grant-inspector">
      <h2>Sharing</h2>
      <p className="muted">Who may use each asset, and under what warrant (read-only).</p>

      {recipes.isLoading ? <EmptyState title="Loading assets…" /> : null}
      {catalogNotWired ? (
        <EmptyState
          title="Sharing not available here"
          detail="This gateway does not expose the recipe catalog, so there are no assets to inspect."
        />
      ) : null}

      {assets.length > 0 ? (
        <div className="chip-row" role="radiogroup" aria-label="Asset">
          {assets.map((a) => (
            <button
              key={a}
              type="button"
              data-testid={`grant-asset-pick-${a}`}
              className={`chip${a === effective ? " chip--active" : ""}`}
              aria-pressed={a === effective}
              onClick={() => onSelectAsset(a)}
            >
              {a}
            </button>
          ))}
        </div>
      ) : null}

      {effective ? <GrantTable grants={grants} /> : null}
    </div>
  );
}

/** The grant rows for the selected asset (loading / error / empty handled inline). */
function GrantTable({ grants }: { grants: ReturnType<typeof useAssetGrants> }) {
  if (grants.isLoading) {
    return <EmptyState title="Loading grants…" />;
  }
  if (grants.isError) {
    const ui = toUiError(grants.error);
    if (ui.kind === "not-wired") {
      return (
        <EmptyState
          title="Grants not available here"
          detail="This gateway does not expose the grants viewer."
        />
      );
    }
    return <ErrorNotice error={ui} onRetry={() => void grants.refetch()} />;
  }
  if (!grants.data || grants.data.grants.length === 0) {
    return <EmptyState title="No grants" detail="No grants are recorded on this asset." />;
  }

  return (
    <table className="data-table" data-testid="grant-table">
      <thead>
        <tr>
          <th>Grantee</th>
          <th>Actions</th>
          <th>Grantor</th>
          <th>Scope</th>
          <th>Status</th>
        </tr>
      </thead>
      <tbody>
        {grants.data.grants.map((g, i) => (
          <tr
            // Grantor+grantee+root uniquely identifies a demo grant; index disambiguates dups.
            key={`${g.grantor}->${g.grantee}-${g.isRoot}-${i}`}
            data-testid={`grant-row-${g.grantee}`}
            className={g.revoked ? "grant-row--revoked" : undefined}
          >
            <td className="mono">{g.grantee}</td>
            <td>{formatActions(g.actions)}</td>
            <td className="mono">{g.grantor}</td>
            <td>{g.runtimeScope}</td>
            <td>
              <span className={`grant-status grant-status--${g.status}`}>
                {grantStatusLabel(g)}
              </span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
