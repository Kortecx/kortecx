import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp, stagger } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { GlowCard } from "../ds/GlowCard";
import { HealthIndicator } from "../metrics/HealthIndicator";
import { GrantInspector } from "../systems/GrantInspector";
import { TeamsPanel } from "../systems/TeamsPanel";

/**
 * The connected gateway + the governance VIEWERS (UI-3): teams (`MembershipLedger`)
 * and sharing (grants / `GrantLedger`), both read-only. Selecting an asset in the
 * sharing inspector also resolves each team member's effective warrant on it
 * (membership ∩ grant) — the kx-fleet thesis, made visible. Managing teams/grants
 * across parties + multi-tenant identity stay cloud (D129).
 */
export function SystemsSection() {
  const { endpoint, wsEndpoint } = useConnection();
  const [selectedTeam, setSelectedTeam] = useState<string | null>(null);
  const [selectedAsset, setSelectedAsset] = useState<string | null>(null);

  return (
    <section className="screen" data-testid="systems-section">
      <h1>Systems</h1>
      <p className="muted">The connected gateway, its liveness, and your teams &amp; sharing.</p>
      <m.div variants={stagger()} initial="hidden" animate="show">
        <GlowCard variants={fadeUp} stripe="var(--primary)" className="systems-facts-card">
          <dl className="facts">
            <dt>Gateway</dt>
            <dd className="mono">{endpoint}</dd>
            <dt>WS bridge</dt>
            <dd className="mono">{wsEndpoint ?? "(derived from endpoint, :50152)"}</dd>
            <dt>Health</dt>
            <dd>
              <HealthIndicator />
            </dd>
          </dl>
        </GlowCard>

        <div className="systems-grid">
          <GlowCard variants={fadeUp} hover={false}>
            <TeamsPanel
              selectedTeam={selectedTeam}
              onSelectTeam={setSelectedTeam}
              assetRef={selectedAsset ?? undefined}
            />
          </GlowCard>
          <GlowCard variants={fadeUp} hover={false}>
            <GrantInspector selectedAsset={selectedAsset} onSelectAsset={setSelectedAsset} />
          </GlowCard>
        </div>
      </m.div>
    </section>
  );
}
