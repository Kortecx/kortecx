/**
 * The durable tools-registry inventory (`DiscoverTools`, PR-6a) — the GOVERNANCE
 * view: every registered tool with its kind, provenance, status, egress host and
 * net-scope, plus an operator deregister control. Built-ins are re-seeded on open
 * and never deregisterable (the control is disabled with a tooltip). Registration
 * grants NO authority (SN-8); listing leaks none. DIALING a registered external
 * MCP server is a Cloud / PR-6b capability — this view records + governs only.
 */

import type { RegisteredTool } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useDeregisterTool, useDiscoverTools } from "../../kx/use-tool-registry";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

/** Accent stripe keyed by the registry kind (display only). */
function kindStripe(kind: string): string {
  if (kind === "Builtin") return "var(--info)";
  if (kind === "Mcp") return "var(--primary)";
  return "var(--violet)";
}

/** A registration-status badge color (Approved = success; pending = warning). */
function statusColor(status: string): string {
  return status === "Approved" ? "var(--success)" : "var(--warning)";
}

export function RegisteredToolsPanel() {
  const { tools, notWired, isLoading, isError, error, refetch } = useDiscoverTools();
  const deregister = useDeregisterTool();
  const deregError = deregister.error ? toUiError(deregister.error) : null;

  if (isLoading) {
    return <EmptyState title="Loading registry…" />;
  }
  if (notWired) {
    return (
      <EmptyState
        title="Tool registry needs a newer gateway"
        detail="This gateway doesn't expose the durable tools registry (an older build)."
      />
    );
  }
  if (isError) {
    return <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />;
  }

  return (
    <div data-testid="tools-registered">
      {/* A deregister failure is typically non-retryable (forbidden / not-found),
          so surface it as a dismissable inline message (the RegisterToolForm
          pattern) — it clears on the next deregister attempt. */}
      {deregError ? (
        <p className="field-error" data-testid="deregister-error" role="alert">
          {deregError.message}
        </p>
      ) : null}
      {tools.length === 0 ? (
        <EmptyState
          title="No tools registered"
          detail="Register an external MCP tool below — or run with KX_SERVE_FS_ROOT to enable the fs-list built-in."
        />
      ) : (
        <m.ul
          className="registry-list"
          data-testid="tools-registered-panel"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {tools.map((tool) => {
            const pending =
              deregister.isPending &&
              deregister.variables?.name === tool.toolName &&
              deregister.variables?.version === tool.toolVersion;
            return (
              <RegistryRow
                key={`${tool.toolName}@${tool.toolVersion}`}
                tool={tool}
                pending={pending}
                onDeregister={() =>
                  deregister.mutate({ name: tool.toolName, version: tool.toolVersion })
                }
              />
            );
          })}
        </m.ul>
      )}
    </div>
  );
}

function RegistryRow({
  tool,
  pending,
  onDeregister,
}: {
  tool: RegisteredTool;
  pending: boolean;
  onDeregister: () => void;
}) {
  return (
    <GlowCard
      className="registry-row"
      stripe={kindStripe(tool.kind)}
      variants={fadeUp}
      {...hoverLift}
    >
      <div className="registry-row__main">
        <div className="registry-row__head">
          {/* No "online" status dot: a registry entry is DECLARED, not fireable
              (dialing is PR-6b). The registrationStatus badge carries the state. */}
          <span
            className="registry-row__name mono"
            data-testid={`registered-tool-${tool.toolName}-${tool.toolVersion}`}
          >
            {tool.toolName}@{tool.toolVersion}
          </span>
          <Badge label={tool.kind} color={kindStripe(tool.kind)} />
          {tool.isBuiltin ? <Badge label="built-in" color="var(--text-2)" /> : null}
          <Badge label={tool.registrationStatus} color={statusColor(tool.registrationStatus)} />
        </div>
        {tool.description ? <p className="registry-row__desc muted">{tool.description}</p> : null}
        <dl className="registry-row__meta">
          <div>
            <dt className="muted">provenance</dt>
            <dd>{tool.provenance}</dd>
          </div>
          <div>
            <dt className="muted">idempotency</dt>
            <dd>{tool.idempotencyClass}</dd>
          </div>
          <div>
            <dt className="muted">egress host</dt>
            <dd className="mono">{tool.serverHost || "—"}</dd>
          </div>
          <div>
            <dt className="muted">net scope</dt>
            <dd className="mono">{tool.netScope}</dd>
          </div>
        </dl>
      </div>
      <button
        type="button"
        className="btn-ghost registry-row__deregister"
        data-testid={`deregister-${tool.toolName}-${tool.toolVersion}`}
        disabled={tool.isBuiltin || pending}
        title={
          tool.isBuiltin
            ? "Built-in tools are re-seeded on start and cannot be deregistered"
            : "Deregister this tool"
        }
        onClick={onDeregister}
      >
        {pending ? "Removing…" : "Deregister"}
      </button>
    </GlowCard>
  );
}
