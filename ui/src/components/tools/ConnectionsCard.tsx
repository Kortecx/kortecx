/**
 * The MCP Connections affordance — an HONEST-DISABLED forward card (GR15
 * don't-fake-gaps / GR19). Registering a tool host (above) records + governs it;
 * DIALING external MCP servers over stdio / Streamable HTTP, with secret-less
 * `CredentialRef` connections, lands in PR-6b. This card states that plainly
 * rather than showing a control with no backend.
 */

export function ConnectionsCard() {
  return (
    <div className="metric-card metric-card--disabled" data-testid="tools-connections-disabled">
      <span className="metric-card__value">
        <span className="chip--soon">PR-6b</span>
      </span>
      <span className="metric-card__label">MCP Connections</span>
      <span className="metric-card__sub">
        Dialing external MCP servers (stdio · Streamable HTTP) and secret-less credential
        connections arrive in the next batch. Tool registration + governance is live above.
      </span>
    </div>
  );
}
