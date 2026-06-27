/** ToolsSection → the Integrations hub (Tools | Connections | Triggers | Secrets).
 *  Pins the tab-routing only; the heavy child panels are stubbed (each has its own
 *  test). Mirrors the ContextSection/SystemsSection section-tabs precedent. */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// Stub the heavy child bodies (each owns its own hooks + tests). Mocks precede the
// section import (vitest hoists them).
vi.mock("../../src/components/tools/RegisteredToolsPanel", () => ({
  RegisteredToolsPanel: () => <div data-testid="stub-registered-tools" />,
}));
vi.mock("../../src/components/tools/RegisterToolForm", () => ({
  RegisterToolForm: () => <div data-testid="stub-register-tool" />,
}));
vi.mock("../../src/components/tools/AutoGrantStatus", () => ({
  AutoGrantStatus: () => <div data-testid="stub-autogrant" />,
}));
vi.mock("../../src/components/tools/ConnectionsPanel", () => ({
  ConnectionsPanel: () => <div data-testid="connections-panel" />,
}));
vi.mock("../../src/components/tools/TriggersPanel", () => ({
  TriggersPanel: () => <div data-testid="triggers-panel" />,
}));
vi.mock("../../src/components/tools/SecretsPanel", () => ({
  SecretsPanel: () => <div data-testid="secrets-panel" />,
}));
vi.mock("../../src/kx/use-toolscout", () => ({
  useToolManifests: () => ({ data: [], isLoading: false, isError: false, error: null }),
  useScoreBundle: () => ({
    mutate: vi.fn(),
    reset: vi.fn(),
    isPending: false,
    data: null,
    error: null,
  }),
}));

import { ToolsSection } from "../../src/components/sections/ToolsSection";

describe("ToolsSection — the Integrations hub tabs", () => {
  it("relabels to Integrations and defaults to the Tools tab", () => {
    render(<ToolsSection tab="tools" />);
    expect(screen.getByTestId("tools-section")).toBeInTheDocument();
    expect(screen.getByTestId("tools-tabs")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Integrations" })).toBeInTheDocument();
    // The Tools tab shows the registry; the other panels are not mounted.
    expect(screen.getByTestId("stub-registered-tools")).toBeInTheDocument();
    expect(screen.getByTestId("tools-tab-tools")).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByTestId("triggers-panel")).toBeNull();
    expect(screen.queryByTestId("secrets-panel")).toBeNull();
    expect(screen.queryByTestId("connections-panel")).toBeNull();
  });

  it("the Connections tab renders the live MCP panel", () => {
    render(<ToolsSection tab="connections" />);
    expect(screen.getByTestId("connections-panel")).toBeInTheDocument();
    expect(screen.queryByTestId("stub-registered-tools")).toBeNull();
  });

  it("the Triggers tab renders the triggers panel", () => {
    render(<ToolsSection tab="triggers" />);
    expect(screen.getByTestId("triggers-panel")).toBeInTheDocument();
    expect(screen.getByTestId("tools-tab-triggers")).toHaveAttribute("aria-pressed", "true");
  });

  it("the Secrets tab renders the secrets panel", () => {
    render(<ToolsSection tab="secrets" />);
    expect(screen.getByTestId("secrets-panel")).toBeInTheDocument();
    expect(screen.getByTestId("tools-tab-secrets")).toHaveAttribute("aria-pressed", "true");
  });

  it("clicking a tab calls onTab with its id", () => {
    const onTab = vi.fn();
    render(<ToolsSection tab="tools" onTab={onTab} />);
    fireEvent.click(screen.getByTestId("tools-tab-triggers"));
    expect(onTab).toHaveBeenCalledWith("triggers");
    fireEvent.click(screen.getByTestId("tools-tab-secrets"));
    expect(onTab).toHaveBeenCalledWith("secrets");
  });
});
