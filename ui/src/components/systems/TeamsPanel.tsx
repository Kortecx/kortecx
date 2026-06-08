import { useEffect } from "react";
import { toUiError } from "../../kx/errors";
import { useTeams } from "../../kx/use-teams";
import { EmptyState } from "../EmptyState";
import { MemberTable } from "./MemberTable";

/**
 * The teams panel: a CHIP picker over the teams the gateway knows (button controls,
 * never a controlled `<select>` — the Playwright `selectOption` gotcha), driving the
 * member table for the selected team. `assetRef` (from the grants inspector) flows
 * through so the member table can resolve each member's warrant on the inspected asset.
 */
export function TeamsPanel({
  selectedTeam,
  onSelectTeam,
  assetRef,
}: {
  selectedTeam: string | null;
  onSelectTeam: (teamId: string) => void;
  assetRef?: string;
}) {
  const teams = useTeams();
  const list = teams.data ?? [];
  const effective =
    selectedTeam && list.some((t) => t.teamId === selectedTeam)
      ? selectedTeam
      : (list[0]?.teamId ?? null);

  // Default the selection to the first team once they load.
  useEffect(() => {
    const first = list[0];
    if (!selectedTeam && first) {
      onSelectTeam(first.teamId);
    }
  }, [selectedTeam, list, onSelectTeam]);

  const notWired = teams.isError && toUiError(teams.error).kind === "not-wired";

  return (
    <div data-testid="teams-panel">
      <h2>Teams</h2>
      {teams.isLoading ? <EmptyState title="Loading teams…" /> : null}
      {notWired ? (
        <EmptyState
          title="Teams not available here"
          detail="This gateway does not expose the teams viewer (an older build). Managing teams across parties is cloud."
        />
      ) : null}
      {teams.isError && !notWired ? (
        <EmptyState title="Couldn't load teams" detail={toUiError(teams.error).message} />
      ) : null}
      {teams.data && list.length === 0 ? (
        <EmptyState title="No teams" detail="No teams have been founded on this gateway yet." />
      ) : null}

      {list.length > 0 ? (
        <>
          <div className="chip-row" role="radiogroup" aria-label="Team">
            {list.map((t) => (
              <button
                key={t.teamId}
                type="button"
                data-testid={`team-pick-${t.teamId}`}
                className={`chip${t.teamId === effective ? " chip--active" : ""}`}
                aria-pressed={t.teamId === effective}
                onClick={() => onSelectTeam(t.teamId)}
              >
                <span className="chip__label">{t.displayName || t.teamId}</span>
                <span className="chip__meta">
                  {t.memberCount} member{t.memberCount === 1 ? "" : "s"}
                </span>
              </button>
            ))}
          </div>
          {effective ? <MemberTable teamId={effective} assetRef={assetRef} /> : null}
        </>
      ) : null}
    </div>
  );
}
