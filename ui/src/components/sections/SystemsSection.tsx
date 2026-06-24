import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp, stagger } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import type { SecurityTab } from "../../router/routes/systems";
import { GlowCard } from "../ds/GlowCard";
import { HealthIndicator } from "../metrics/HealthIndicator";
import { GrantInspector } from "../systems/GrantInspector";
import { TeamsPanel } from "../systems/TeamsPanel";
import { PoliciesSection } from "./PoliciesSection";

const TABS: ReadonlyArray<{ id: SecurityTab; label: string }> = [
  { id: "teams", label: "Teams & grants" },
  { id: "policies", label: "Policies" },
];

/**
 * Security (POC-5c / D168): policy, RBAC & agent-access for the connected gateway.
 * Two URL-addressable tabs:
 *
 * 1. **Teams & grants** — the governance VIEWERS (UI-3): teams (`MembershipLedger`)
 *    and sharing (grants / `GrantLedger`), read-only. Selecting an asset resolves
 *    each member's effective warrant (membership ∩ grant) — the kx-fleet thesis made
 *    visible. Managing teams/grants across parties is Cloud (D129).
 * 2. **Policies** — the OSS per-App agent-write lock gate ({@link PoliciesSection}):
 *    a locked App refuses agentic in-CAS edits at the runtime advance() chokepoint.
 *
 * Pure renderer: tab state rides the route's validated search.
 */
export function SystemsSection({
  tab = "teams",
  onTab,
}: {
  tab?: SecurityTab;
  onTab?: (tab: SecurityTab) => void;
} = {}) {
  return (
    <section className="screen" data-testid="systems-section">
      <div className="section-head">
        <div>
          <h1>Security</h1>
          <p className="muted">
            Teams, grants & per-App policies on the connected gateway. Cross-party RBAC & authoring
            are a managed-cloud capability.
          </p>
        </div>
      </div>

      <fieldset className="view-toggle" aria-label="Security view" data-testid="security-tabs">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            data-testid={`security-tab-${t.id}`}
            aria-pressed={tab === t.id}
            onClick={() => onTab?.(t.id)}
          >
            {t.label}
          </button>
        ))}
      </fieldset>

      {tab === "policies" ? <PoliciesSection /> : <TeamsGrantsView />}
    </section>
  );
}

/** The teams + grants viewers (UI-3) — the default Security tab. */
function TeamsGrantsView() {
  const { endpoint, wsEndpoint } = useConnection();
  const [selectedTeam, setSelectedTeam] = useState<string | null>(null);
  const [selectedAsset, setSelectedAsset] = useState<string | null>(null);

  return (
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
  );
}
