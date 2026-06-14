import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

// Mutable telemetry return, swapped per test (hoisted above the vi.mock factory).
const tel = vi.hoisted(() => ({
  ret: {
    rows: [] as Array<{ outputTokens: number | null; startedUnixMs: number }>,
    notWired: false,
  },
}));
vi.mock("../../src/kx/use-telemetry", () => ({ useTelemetry: () => tel.ret }));

import { TokenUsageFooter } from "../../src/components/shell/TokenUsageFooter";

function startOfToday(): number {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

afterEach(() => {
  tel.ret = { rows: [], notWired: false };
});

describe("TokenUsageFooter (PR-B real-or-honest-empty)", () => {
  it("honest-empty when there is no model telemetry (no fabricated 0)", () => {
    render(<TokenUsageFooter />);
    expect(screen.getByTestId("token-usage")).toBeInTheDocument();
    expect(screen.getByTestId("token-usage-empty")).toHaveTextContent("no model telemetry");
  });

  it("honest-empty (distinct caption) when the telemetry RPC is not wired", () => {
    tel.ret = { rows: [], notWired: true };
    render(<TokenUsageFooter />);
    expect(screen.getByTestId("token-usage-empty")).toHaveTextContent("no usage telemetry");
  });

  it("sums today's REAL output tokens (output-only, no fabricated limit)", () => {
    const today = startOfToday() + 1000;
    tel.ret = {
      notWired: false,
      rows: [
        { outputTokens: 1200, startedUnixMs: today },
        { outputTokens: 280, startedUnixMs: today },
        { outputTokens: null, startedUnixMs: today }, // a PURE mote — contributes 0
      ],
    };
    render(<TokenUsageFooter />);
    expect(screen.queryByTestId("token-usage-empty")).not.toBeInTheDocument();
    expect(screen.getByTestId("token-usage")).toHaveTextContent("1,480");
  });

  it("excludes rows from before today (the 'today' window)", () => {
    const yesterday = startOfToday() - 60_000;
    tel.ret = { notWired: false, rows: [{ outputTokens: 9999, startedUnixMs: yesterday }] };
    render(<TokenUsageFooter />);
    // Nothing today → honest-empty, never yesterday's count.
    expect(screen.getByTestId("token-usage-empty")).toBeInTheDocument();
  });
});
