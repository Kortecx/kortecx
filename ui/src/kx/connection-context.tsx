/**
 * The gateway connection. Holds the live {@link KxClientBase} (or null) plus the
 * connect/disconnect machine.
 *
 * SECURITY: the bearer token is held ONLY inside the constructed client instance
 * (in memory). It is never written to storage, never placed in the bundle, and the
 * context never hands it back out. Only the (non-secret) endpoint is persisted.
 */

import { DEFAULT_ENDPOINT, KxClient } from "@kortecx/sdk/web";
import type { KxClientBase } from "@kortecx/sdk/web";
import { useQueryClient } from "@tanstack/react-query";
import { type ReactNode, createContext, useCallback, useContext, useMemo, useState } from "react";
import { type UiError, toUiError } from "./errors";

/** Build a client for an endpoint + optional token/ws-bridge. Injectable for tests. */
export type ClientFactory = (
  endpoint: string,
  opts: { token?: string; wsEndpoint?: string },
) => KxClientBase;

const defaultFactory: ClientFactory = (endpoint, opts) => new KxClient(endpoint, opts);

export type ConnectionStatus = "disconnected" | "connecting" | "connected";

export interface ConnectionState {
  readonly status: ConnectionStatus;
  readonly endpoint: string;
  /**
   * The explicit WS-bridge endpoint for the live event tail (`wsEvents`), or null
   * to derive it from the gRPC endpoint (conventional port 50152). Non-secret.
   */
  readonly wsEndpoint: string | null;
  readonly client: KxClientBase | null;
  readonly error: UiError | null;
  /** Connect + probe; resolves `true` on success, `false` on a surfaced error. */
  connect(endpoint: string, token?: string, wsEndpoint?: string): Promise<boolean>;
  disconnect(): void;
}

const ENDPOINT_KEY = "kortecx.ui.endpoint";
const WS_ENDPOINT_KEY = "kortecx.ui.wsEndpoint";

/** Exported so tests can provide a connected state directly (DI). */
export const ConnectionContext = createContext<ConnectionState | null>(null);

export interface KxConnectionProviderProps {
  children: ReactNode;
  /** Injectable client builder; defaults to the gRPC-web browser client. */
  createClient?: ClientFactory;
  /** Initial endpoint (defaults to the persisted value, then the SDK default). */
  initialEndpoint?: string;
}

function loadEndpoint(fallback: string): string {
  try {
    return localStorage.getItem(ENDPOINT_KEY) ?? fallback;
  } catch {
    return fallback;
  }
}

function persistEndpoint(endpoint: string): void {
  try {
    localStorage.setItem(ENDPOINT_KEY, endpoint);
  } catch {
    /* best-effort; storage may be unavailable (private mode / SSR) */
  }
}

function loadWsEndpoint(): string | null {
  try {
    return localStorage.getItem(WS_ENDPOINT_KEY);
  } catch {
    return null;
  }
}

function persistWsEndpoint(wsEndpoint: string | null): void {
  try {
    if (wsEndpoint) {
      localStorage.setItem(WS_ENDPOINT_KEY, wsEndpoint);
    } else {
      localStorage.removeItem(WS_ENDPOINT_KEY);
    }
  } catch {
    /* best-effort */
  }
}

export function KxConnectionProvider({
  children,
  createClient = defaultFactory,
  initialEndpoint,
}: KxConnectionProviderProps) {
  const queryClient = useQueryClient();
  const [status, setStatus] = useState<ConnectionStatus>("disconnected");
  const [endpoint, setEndpoint] = useState<string>(
    () => initialEndpoint ?? loadEndpoint(DEFAULT_ENDPOINT),
  );
  const [wsEndpoint, setWsEndpoint] = useState<string | null>(() => loadWsEndpoint());
  const [client, setClient] = useState<KxClientBase | null>(null);
  const [error, setError] = useState<UiError | null>(null);

  const connect = useCallback(
    async (ep: string, token?: string, ws?: string): Promise<boolean> => {
      setStatus("connecting");
      setError(null);
      const wsTrim = ws?.trim() ? ws.trim() : undefined;
      // The token is passed straight into the client (memory only) and dropped here.
      const candidate = createClient(ep, {
        ...(token ? { token } : {}),
        ...(wsTrim ? { wsEndpoint: wsTrim } : {}),
      });
      try {
        // Cheap unary probe. A real gRPC answer (even UNIMPLEMENTED) = reachable + authorized.
        await candidate.listSignatures();
      } catch (e) {
        const ui = toUiError(e);
        if (ui.kind !== "not-wired") {
          candidate.close();
          setClient(null);
          setStatus("disconnected");
          setError(ui);
          return false;
        }
      }
      setClient(candidate);
      setEndpoint(ep);
      setWsEndpoint(wsTrim ?? null);
      setStatus("connected");
      persistEndpoint(ep);
      persistWsEndpoint(wsTrim ?? null);
      return true;
    },
    [createClient],
  );

  const disconnect = useCallback((): void => {
    client?.close();
    setClient(null);
    setStatus("disconnected");
    setError(null);
    queryClient.clear(); // drop any cached projections from the old endpoint
  }, [client, queryClient]);

  const value = useMemo<ConnectionState>(
    () => ({ status, endpoint, wsEndpoint, client, error, connect, disconnect }),
    [status, endpoint, wsEndpoint, client, error, connect, disconnect],
  );

  return <ConnectionContext.Provider value={value}>{children}</ConnectionContext.Provider>;
}

export function useConnection(): ConnectionState {
  const ctx = useContext(ConnectionContext);
  if (ctx === null) {
    throw new Error("useConnection must be used within <KxConnectionProvider>");
  }
  return ctx;
}
