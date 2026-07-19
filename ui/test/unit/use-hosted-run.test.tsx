/**
 * Bug D — the hosted Run control. `StartHostedApp` returns while the app is still
 * materializing (its URL empty), so the old "open on success if url" click silently did nothing.
 * `useHostedRun` must open the live tab exactly when the app is actually running, and never on a
 * still-materializing start. These guard that behavior + the absolute-URL open, without leaning
 * on the 3s status poll (which the live-proof exercises end-to-end).
 */

import type { HostedAppStatus } from "@kortecx/sdk/web";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";

const startImpl = vi.fn();
const statusImpl = vi.fn();

vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({
    client: {
      startHostedApp: (...a: unknown[]) => startImpl(...a),
      getHostedAppStatus: (...a: unknown[]) => statusImpl(...a),
    },
    endpoint: "http://127.0.0.1:50151",
    status: "connected",
  }),
}));

import { useHostedRun } from "../../src/kx/use-hosted-app";

function status(over: Partial<HostedAppStatus>): HostedAppStatus {
  return {
    handle: "apps/local/x",
    state: "stopped",
    url: "",
    recentLogs: [],
    framework: "vite_react",
    port: 0,
    detail: "",
    ...over,
  };
}

function wrapper({ children }: { children: ReactNode }) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

afterEach(() => {
  vi.restoreAllMocks();
  startImpl.mockReset();
  statusImpl.mockReset();
});

it("opens the live tab (absolute URL) when the app is already running on start", async () => {
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  startImpl.mockResolvedValue(
    status({ state: "running", url: "http://127.0.0.1:5555/", port: 5555 }),
  );
  statusImpl.mockResolvedValue(status({ state: "stopped" }));

  const { result } = renderHook(() => useHostedRun("apps/local/x"), { wrapper });
  act(() => {
    result.current.run();
  });

  await waitFor(() => expect(open).toHaveBeenCalled());
  expect(open).toHaveBeenCalledWith("http://127.0.0.1:5555/", "_blank", "noopener");
});

it("does NOT open a tab when start returns while still materializing (URL empty)", async () => {
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  startImpl.mockResolvedValue(status({ state: "materializing" }));
  statusImpl.mockResolvedValue(status({ state: "materializing" }));

  const { result } = renderHook(() => useHostedRun("apps/local/x"), { wrapper });
  act(() => {
    result.current.run();
  });

  await new Promise((r) => setTimeout(r, 50));
  expect(open).not.toHaveBeenCalled();
});
