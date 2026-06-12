import { m } from "framer-motion";
import { fadeUp, stagger } from "../../app/motion";
import { useTheme } from "../../app/use-theme";
import { useConnection } from "../../kx/connection-context";
import { THEME_PREFERENCES } from "../../lib/theme";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

/**
 * Settings/Profile (W1 shell placeholder per the section taxonomy): the connection
 * profile + console appearance controls. No new RPCs; preferences persist locally
 * in this browser (theme via lib/theme.ts), never on the gateway.
 */
export function SettingsSection() {
  const { status, endpoint, wsEndpoint } = useConnection();
  const { preference, resolved, setPreference } = useTheme();
  const connected = status === "connected";
  return (
    <section className="screen" data-testid="settings-section">
      <h1>Settings</h1>
      <p className="muted">
        Console preferences & the connection profile. Everything here lives in this browser —
        nothing is stored on the gateway.
      </p>
      <m.div className="settings-grid" variants={stagger()} initial="hidden" animate="show">
        <GlowCard variants={fadeUp}>
          <h2>Connection</h2>
          <dl className="facts">
            <dt>Status</dt>
            <dd>
              <Badge
                label={status}
                color={connected ? "var(--success)" : "var(--error)"}
                dot
                pulse={connected}
              />
            </dd>
            <dt>Endpoint</dt>
            <dd className="mono">{endpoint}</dd>
            <dt>WS bridge</dt>
            <dd className="mono">{wsEndpoint ?? "derived from the endpoint"}</dd>
            <dt>Bearer token</dt>
            <dd>kept in memory only — never persisted</dd>
          </dl>
        </GlowCard>
        <GlowCard variants={fadeUp}>
          <h2>Appearance</h2>
          <dl className="facts">
            <dt>Theme</dt>
            <dd>
              {/* chip buttons, not a <select> — the recorded controlled-select e2e gotcha */}
              <div className="chip-row">
                {THEME_PREFERENCES.map((pref) => (
                  <button
                    key={pref}
                    type="button"
                    className={`chip${preference === pref ? " chip--active" : ""}`}
                    aria-pressed={preference === pref}
                    data-testid={`theme-chip-${pref}`}
                    onClick={() => setPreference(pref)}
                  >
                    <span className="chip__label">{pref}</span>
                  </button>
                ))}
              </div>
              <span className="muted" data-testid="theme-resolved">
                rendering: kortecx {resolved}
              </span>
            </dd>
            <dt>Type</dt>
            <dd>Geist Sans · Geist Mono</dd>
            <dt>Sidebar</dt>
            <dd>collapse persists per browser (the sidebar hamburger)</dd>
            <dt>Motion</dt>
            <dd>honors your reduced-motion preference</dd>
          </dl>
        </GlowCard>
      </m.div>
    </section>
  );
}
