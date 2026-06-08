import { type FormEvent, useState } from "react";
import { isNonloopbackPlaintext, validateEndpoint } from "../lib/endpoint";

export interface ConnectionFormProps {
  initialEndpoint: string;
  connecting: boolean;
  onConnect: (endpoint: string, token: string | undefined) => void;
}

/**
 * Endpoint + optional bearer token. The token is held in local component state
 * (memory) and handed to `onConnect`; it is NEVER persisted. A cleartext warning
 * shows when a token would cross plaintext http:// to a non-loopback host.
 */
export function ConnectionForm({ initialEndpoint, connecting, onConnect }: ConnectionFormProps) {
  const [endpoint, setEndpoint] = useState(initialEndpoint);
  const [token, setToken] = useState("");
  const endpointError = validateEndpoint(endpoint);
  const plaintextWarning = token.trim() !== "" && isNonloopbackPlaintext(endpoint.trim());

  function submit(e: FormEvent<HTMLFormElement>): void {
    e.preventDefault();
    if (endpointError !== null) {
      return;
    }
    const t = token.trim();
    onConnect(endpoint.trim(), t === "" ? undefined : t);
  }

  return (
    <form className="connect-form" onSubmit={submit} data-testid="connection-form">
      <label htmlFor="endpoint">Gateway endpoint</label>
      <input
        id="endpoint"
        name="endpoint"
        type="text"
        value={endpoint}
        onChange={(e) => setEndpoint(e.target.value)}
        autoComplete="off"
        spellCheck={false}
      />
      {endpointError !== null ? (
        <p className="field-error" role="alert">
          {endpointError}
        </p>
      ) : null}

      <label htmlFor="token">Bearer token (optional)</label>
      <input
        id="token"
        name="token"
        type="password"
        value={token}
        onChange={(e) => setToken(e.target.value)}
        autoComplete="off"
        placeholder="blank for --dev-allow-local"
      />
      {plaintextWarning ? (
        <output className="field-warning" data-testid="plaintext-warning">
          ⚠ This token would travel in cleartext to a non-loopback host. Use an https:// endpoint
          (TLS) for remote gateways.
        </output>
      ) : null}

      <button type="submit" disabled={connecting || endpointError !== null}>
        {connecting ? "Connecting…" : "Connect"}
      </button>
    </form>
  );
}
