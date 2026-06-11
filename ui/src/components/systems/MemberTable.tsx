import { toUiError } from "../../kx/errors";
import { useTeamMembers } from "../../kx/use-teams";
import { formatActionCaps, roleBadgeKind, warrantRows } from "../../lib/team-format";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";

/**
 * The members of one team: party, role (+ a delegate/member badge), and action caps.
 * When `assetRef` is set (an asset is selected in the grants inspector), each member
 * also shows the warrant they would RESOLVE on that asset through the team membership
 * (membership ∩ grant, ⊆ the team) — the kx-fleet composition, made visible.
 */
export function MemberTable({ teamId, assetRef }: { teamId: string; assetRef?: string }) {
  const members = useTeamMembers(teamId, assetRef);

  if (members.isLoading) {
    return <EmptyState title="Loading members…" />;
  }
  if (members.isError) {
    return <ErrorNotice error={toUiError(members.error)} onRetry={() => void members.refetch()} />;
  }
  if (!members.data || members.data.members.length === 0) {
    return <EmptyState title="No members" detail="This team has no active members." />;
  }

  return (
    <div data-testid="member-table">
      <p className="muted">
        Owner <span className="mono">{members.data.owner}</span>
        {assetRef ? (
          <>
            {" · resolving warrants on "}
            <span className="mono">{assetRef}</span>
          </>
        ) : null}
      </p>
      <table className="data-table">
        <thead>
          <tr>
            <th>Member</th>
            <th>Role</th>
            <th>Caps</th>
            {assetRef ? <th>Resolved warrant</th> : null}
          </tr>
        </thead>
        <tbody>
          {members.data.members.map((m) => (
            <tr key={m.party} data-testid={`member-row-${m.party}`}>
              <td className="mono">{m.party}</td>
              <td>
                <span className={`role-badge role-badge--${roleBadgeKind(m)}`}>{m.role}</span>
              </td>
              <td>{formatActionCaps(m.actionCaps)}</td>
              {assetRef ? (
                <td data-testid={`member-warrant-${m.party}`}>
                  {m.resolvedWarrant ? (
                    <>
                      <span className="status-dot status-dot--online" aria-hidden="true" />
                      <dl className="warrant-rows">
                        {warrantRows(m.resolvedWarrant).map((row) => (
                          <div className="warrant-row" key={row.label}>
                            <dt>{row.label}</dt>
                            <dd className="mono">{row.value}</dd>
                          </div>
                        ))}
                      </dl>
                    </>
                  ) : (
                    <>
                      <span className="status-dot status-dot--offline" aria-hidden="true" />
                      <span className="muted">— (no path resolves)</span>
                    </>
                  )}
                </td>
              ) : null}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
