import { MoteTelemetryRow } from "@kortecx/sdk/web";
import { render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";

// The dashboard renders TanStack <Link>s; stub them to plain <a> so the section
// tests in isolation (real router navigation is covered by e2e/dashboard.spec.ts).
vi.mock("@tanstack/react-router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tanstack/react-router")>();
  return {
    ...actual,
    Link: ({ to, params, activeProps, children, ...rest }: any) =>
      React.createElement("a", { href: typeof to === "string" ? to : "#", ...rest }, children),
  };
});

import { DashboardSection } from "../../src/components/sections/DashboardSection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

describe("DashboardSection", () => {
  it("renders honest KPIs derived from real RPCs (no fabricated cards)", async () => {
    const mock = makeMockClient({
      listMoteTelemetry: async () => ({
        rows: [
          new MoteTelemetryRow("m1", "", 10, null, 5, "qwen3", "", 0, 2),
          new MoteTelemetryRow("m2", "", 30, null, 7, "qwen3", "", 0, 1),
        ],
        hasMore: false,
      }),
      listModels: async () => [
        {
          modelId: "qwen3",
          modalities: ["text"],
          description: "",
          serving: true,
          contextLen: 4096,
        },
        { modelId: "x", modalities: ["text"], description: "", serving: false, contextLen: 4096 },
      ],
    });
    render(<DashboardSection />, { wrapper: connectedWrapper(mock.client) });
    expect(screen.getByTestId("dashboard-section")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("dashboard-kpis")).toBeInTheDocument());
    // Output tokens KPI = 5 + 7 = 12 (real, summed from telemetry — never fabricated).
    await waitFor(() => expect(screen.getByText("12")).toBeInTheDocument());
    // The honest window qualifier appears (not implied all-time).
    expect(screen.getAllByText(/over last 2 motes/i).length).toBeGreaterThan(0);
    // GR15: none of the reference app's fabricated cards leak in.
    expect(screen.queryByText(/active agents/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/success rate/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/tasks today/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/\$/)).not.toBeInTheDocument();
  });

  it("shows an honest-empty recent-runs state when there are no runs", async () => {
    const mock = makeMockClient({});
    render(<DashboardSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByText(/No runs yet/i)).toBeInTheDocument());
  });
});
