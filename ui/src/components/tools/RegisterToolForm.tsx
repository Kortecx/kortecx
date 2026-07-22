/**
 * Register a declarative EXTERNAL MCP tool into the durable registry (`RegisterTool`,
 * PR-6a). The server SSRF-vets `serverHost`, derives the `toolId` + capability, and
 * stores it (the client never names/forges identity — SN-8). Registration grants NO
 * authority and does NOT dial the host (dialing is a Cloud / PR-6b capability). An
 * internal / link-local / metadata host is refused (`permission_denied`).
 *
 * The idempotency class is chosen via CHIP buttons (never a controlled `<select>` —
 * the UI-3 React-controlled-select e2e gotcha).
 */

import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useRegisterTool } from "../../kx/use-tool-registry";
import { GlowCard } from "../ds/GlowCard";

/** The closed idempotency-class vocabulary (mirrors the registry's `IdempotencyClass`). */
const IDEMPOTENCY_CLASSES = ["Token", "Readback", "Staged", "AtLeastOnce"] as const;
type IdempotencyClass = (typeof IDEMPOTENCY_CLASSES)[number];

export function RegisterToolForm() {
  const [name, setName] = useState("");
  const [version, setVersion] = useState("1");
  const [serverHost, setServerHost] = useState("");
  const [description, setDescription] = useState("");
  const [idempotency, setIdempotency] = useState<IdempotencyClass>("Readback");
  const register = useRegisterTool();

  const canSubmit =
    name.trim().length > 0 && version.trim().length > 0 && serverHost.trim().length > 0;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) {
      return;
    }
    register.mutate({
      name: name.trim(),
      version: version.trim(),
      serverHost: serverHost.trim(),
      description: description.trim(),
      idempotencyClass: idempotency,
    });
  };

  const err = register.error ? toUiError(register.error) : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="register-tool-panel">
      <h2>Register an external MCP tool</h2>
      <p className="muted">
        Records a declarative HTTP tool + its SSRF-vetted egress host in the durable registry.
        Grants no authority (SN-8); a tool fires only under a server-issued warrant. Registering
        records the tool; executing its HTTP egress at run is a Cloud capability. (To dial an MCP
        server locally, use <strong>Connections</strong> — that ships in OSS.)
      </p>
      <form onSubmit={onSubmit} className="register-tool-form">
        <div className="register-tool-form__row">
          <input
            type="text"
            data-testid="register-tool-name"
            placeholder="tool name (e.g. web-search)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="Tool name"
          />
          <input
            type="text"
            data-testid="register-tool-version"
            placeholder="version"
            value={version}
            onChange={(e) => setVersion(e.target.value)}
            aria-label="Tool version"
          />
        </div>
        <input
          type="text"
          data-testid="register-tool-host"
          placeholder="server host (host[:port], e.g. mcp.example.com:443)"
          value={serverHost}
          onChange={(e) => setServerHost(e.target.value)}
          aria-label="Server host"
        />
        <input
          type="text"
          data-testid="register-tool-description"
          placeholder="description (optional)"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          aria-label="Description"
        />
        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Idempotency class</legend>
          <div className="chip-row">
            {IDEMPOTENCY_CLASSES.map((cls) => (
              <button
                key={cls}
                type="button"
                className={`chip${idempotency === cls ? " chip--active" : ""}`}
                data-testid={`register-tool-idempotency-${cls}`}
                aria-pressed={idempotency === cls}
                onClick={() => setIdempotency(cls)}
              >
                <span className="chip__label">{cls}</span>
              </button>
            ))}
          </div>
        </fieldset>
        <button
          type="submit"
          data-testid="register-tool-submit"
          disabled={register.isPending || !canSubmit}
        >
          {register.isPending ? "Registering…" : "Register tool"}
        </button>
      </form>

      {err ? (
        <p className="field-error" data-testid="register-tool-error" role="alert">
          {err.kind === "forbidden" ? "Host not permitted: " : ""}
          {err.message}
        </p>
      ) : null}
      {register.isSuccess ? (
        <p className="register-tool__result" data-testid="register-tool-result">
          Registered — server-derived tool id <code className="mono">{register.data}</code>
        </p>
      ) : null}
    </GlowCard>
  );
}
