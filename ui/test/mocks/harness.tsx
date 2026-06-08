/** Test wrappers: a connected connection context + a query client. */

import type { KxClientBase } from "@kortecx/sdk/web";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { vi } from "vitest";
import { ConnectionContext, type ConnectionState } from "../../src/kx/connection-context";

export function makeTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { retry: false },
    },
  });
}

/** A wrapper that presents the app as already CONNECTED to `client`. */
export function connectedWrapper(client: KxClientBase, endpoint = "http://127.0.0.1:50151") {
  const qc = makeTestQueryClient();
  const value: ConnectionState = {
    status: "connected",
    endpoint,
    wsEndpoint: null,
    client,
    error: null,
    connect: vi.fn(async () => true),
    disconnect: vi.fn(),
  };
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <ConnectionContext.Provider value={value}>{children}</ConnectionContext.Provider>
      </QueryClientProvider>
    );
  };
}
