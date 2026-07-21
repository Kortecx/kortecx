/**
 * D213 Experience lane — the hosted app's run surface on the App detail page.
 *
 * Renders the parts of `HostedAppStatus` that the server has always populated and the
 * console has never shown: the framework, the loopback port, the live URL as a real
 * link, and the supervisor's `recentLogs` ring. When a hosted app fails to install or
 * start, those logs carry npm's or the bundler's own error — without them the console
 * could say only "failed", which is the difference between a user fixing their app and
 * filing a bug.
 *
 * The URL is an ABSOLUTE loopback origin (`http://127.0.0.1:<port>/`), not a
 * console-relative path: there is no reverse proxy, so this opens in a new tab and could
 * not be an iframe against the console's own origin.
 */

import { useHostedAppStatus } from "../../kx/use-hosted-app";
import { EmptyState } from "../EmptyState";

export function HostedRunPanel({ handle }: { handle: string }) {
  const { status, notWired } = useHostedAppStatus(handle, true);

  if (notWired) {
    return (
      <EmptyState
        title="Hosted apps aren't available on this gateway"
        detail="This server was built without the hosted-apps feature, so it cannot materialize, install or serve a web app. The project files above are still real and editable."
      />
    );
  }
  if (!status) {
    return <EmptyState title="Loading app status…" />;
  }

  const running = status.state === "running" && status.url !== "";
  return (
    <section className="hosted-panel" data-testid={`app-detail-hosted-panel-${handle}`}>
      <dl className="hosted-panel__facts">
        <div>
          <dt className="muted">Status</dt>
          <dd data-testid="hosted-panel-state">{status.state}</dd>
        </div>
        <div>
          <dt className="muted">Framework</dt>
          <dd className="mono" data-testid="hosted-panel-framework">
            {status.framework || "—"}
          </dd>
        </div>
        <div>
          <dt className="muted">Serving</dt>
          {/* dev = hot reload over source; production = a built artifact. The server
              echoes this so the console never has to infer it from the state sequence. */}
          <dd data-testid="hosted-panel-serve-mode">
            {status.serveMode === "production" ? "production build" : "dev (hot reload)"}
          </dd>
        </div>
        <div>
          <dt className="muted">Port</dt>
          <dd className="mono" data-testid="hosted-panel-port">
            {status.port > 0 ? status.port : "—"}
          </dd>
        </div>
      </dl>

      {running ? (
        <p className="hosted-panel__url">
          <a
            href={status.url}
            target="_blank"
            rel="noreferrer noopener"
            className="mono"
            data-testid="hosted-panel-url"
          >
            {status.url}
          </a>{" "}
          <span className="muted">— opens in a new tab (a loopback origin, not proxied)</span>
        </p>
      ) : null}

      {status.detail ? (
        <output className="muted" data-testid="hosted-panel-detail">
          {status.detail}
        </output>
      ) : null}

      {/* The supervisor's log ring: captured server-side since the lane shipped, shown
          nowhere until now. Newest last, matching how the process wrote them. */}
      {status.recentLogs.length > 0 ? (
        <details className="hosted-panel__logs" open={status.state === "failed"}>
          <summary>Server log ({status.recentLogs.length} lines)</summary>
          <pre className="mono" data-testid="hosted-panel-logs">
            {status.recentLogs.join("\n")}
          </pre>
        </details>
      ) : (
        <p className="muted" data-testid="hosted-panel-no-logs">
          No server output yet — logs appear once the app is started.
        </p>
      )}
    </section>
  );
}
