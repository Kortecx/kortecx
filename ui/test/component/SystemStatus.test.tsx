import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

// Mutable health return, swapped per test.
const hp = vi.hoisted(() => ({ data: "live" as "live" | "degraded" | "down" | undefined }));
vi.mock("../../src/kx/use-health", () => ({ useHealth: () => ({ data: hp.data }) }));

import { SystemStatus } from "../../src/components/shell/SystemStatus";

afterEach(() => {
  hp.data = "live";
});

describe("SystemStatus (PR-B real-only health box)", () => {
  it("shows the live tone + pulse when the gateway is live", () => {
    render(<SystemStatus />);
    const box = screen.getByTestId("system-status");
    expect(box).toHaveAttribute("data-health", "live");
    expect(box).toHaveTextContent("Live");
    expect(box.querySelector(".status-dot--online")).not.toBeNull();
    expect(box.querySelector(".status-dot--pulse")).not.toBeNull();
  });

  it("shows degraded (no pulse) without fabricating agent/uptime fields", () => {
    hp.data = "degraded";
    render(<SystemStatus />);
    const box = screen.getByTestId("system-status");
    expect(box).toHaveAttribute("data-health", "degraded");
    expect(box).toHaveTextContent("Degraded");
    expect(box.querySelector(".status-dot--busy")).not.toBeNull();
    expect(box.querySelector(".status-dot--pulse")).toBeNull();
    // honest: only the health word — no active-agents / uptime (no RPC).
    expect(box.textContent).toBe("Degraded");
  });

  it("shows down when the gateway is unreachable", () => {
    hp.data = "down";
    render(<SystemStatus />);
    const box = screen.getByTestId("system-status");
    expect(box).toHaveAttribute("data-health", "down");
    expect(box.querySelector(".status-dot--error")).not.toBeNull();
  });

  it("defaults to live when health is undefined (mirrors ConnectionStatus)", () => {
    hp.data = undefined;
    render(<SystemStatus />);
    expect(screen.getByTestId("system-status")).toHaveAttribute("data-health", "live");
  });
});
