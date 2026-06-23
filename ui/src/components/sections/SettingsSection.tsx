import { m } from "framer-motion";
import { fadeUp, stagger } from "../../app/motion";
import { useTheme } from "../../app/use-theme";
import { useConnection } from "../../kx/connection-context";
import { useServerInfo } from "../../kx/use-server-info";
import { THEME_PREFERENCES } from "../../lib/theme";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

/**
 * Settings (W1 shell + POC-1 Workspace): the connection profile + console appearance
 * (browser-local), plus a READ-ONLY "Workspace" view of how the gateway is configured
 * (`GetServerInfo` — non-secret facts, governed by your authenticated session).
 */
export function SettingsSection() {
  const { status, endpoint, wsEndpoint } = useConnection();
  const { preference, resolved, setPreference } = useTheme();
  const info = useServerInfo();
  const connected = status === "connected";
  return (
    <section className="screen" data-testid="settings-section">
      <h1>Settings</h1>
      <p className="muted">
        Console preferences live in this browser. The <strong>Workspace</strong> card is a read-only
        view of how the gateway is configured — non-secret facts only.
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
          <h2>Workspace</h2>
          {info.isError ? (
            <p className="muted" data-testid="workspace-degraded">
              This gateway doesn't expose server info (an older build), or your session isn't
              authenticated — connect with a bearer token to view the workspace configuration.
            </p>
          ) : info.isLoading || info.data === undefined ? (
            <p className="muted">Loading…</p>
          ) : (
            <dl className="facts" data-testid="workspace-facts">
              <dt>Model</dt>
              <dd className="mono">{info.data.modelId || "none — model-less serve"}</dd>
              <dt>gRPC</dt>
              <dd className="mono">{info.data.listenAddr}</dd>
              <dt>WS bridge</dt>
              <dd className="mono">{info.data.wsAddr}</dd>
              <dt>Console</dt>
              <dd className="mono">{info.data.consoleAddr || "disabled"}</dd>
              <dt>Metrics</dt>
              <dd className="mono">{info.data.metricsAddr || "off"}</dd>
              <dt>Content store</dt>
              <dd className="mono">{info.data.contentRoot}</dd>
              <dt>Journal</dt>
              <dd className="mono">{info.data.journalPath}</dd>
              <dt>Security</dt>
              <dd>
                auth {info.data.authMode} · TLS {info.data.tlsEnabled ? "on" : "off"} · CORS{" "}
                {info.data.corsOrigins.length > 0
                  ? info.data.corsOrigins.join(", ")
                  : "deny-by-default"}
              </dd>
              <dt>Features</dt>
              <dd>
                {[
                  info.data.featureInference && "inference",
                  info.data.featureHnsw && "datasets",
                  info.data.featureConsole && "console",
                  info.data.featureVision && "vision",
                ]
                  .filter(Boolean)
                  .join(" · ") || "core only"}
              </dd>
              <dt>Audit log</dt>
              <dd>{info.data.auditLogEnabled ? "on" : "off"}</dd>
              <dt>Agentic budget</dt>
              <dd data-testid="settings-agentic-budget">
                up to {info.data.reactMaxTurns} model turns · {info.data.reactMaxToolCalls} tool
                calls <span className="muted">(default; per-run overridable)</span>
              </dd>
            </dl>
          )}
          <span className="muted">
            Read-only · governed by your authenticated session · never includes a secret.
          </span>
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
