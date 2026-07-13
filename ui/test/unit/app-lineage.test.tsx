/**
 * POC-5d / redesign: the single-App Lineage pane — now a clean, READ-ONLY diagram
 * (a static dagre layout of node cards + SVG connectors, no reactflow editor). The
 * pixel layout is covered by the browser E2E; here the logic is asserted: the pane
 * VIEWS + offers Run with NO authoring controls (relocated to Workflows), the parsed
 * blueprint steps land on the diagram, and an un-round-trippable (exec) blueprint
 * still renders read-only.
 */

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

let ENVELOPE: Record<string, unknown> = {
  blueprint: { seed: 0, steps: [{ kind: "model", model_id: "m", prompt: "hi" }] },
};

vi.mock("../../src/kx/use-apps", () => ({
  useApp: () => ({
    data: { summary: { name: "X", locked: false }, envelope: ENVELOPE },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useSaveApp: () => ({
    mutate: vi.fn(),
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
  useRunApp: () => ({
    mutate: vi.fn(),
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
}));
vi.mock("../../src/kx/use-models", () => ({
  useModels: () => ({ models: [{ id: "m", serving: true }], unsupported: false }),
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));

import { AppLineageSection } from "../../src/components/sections/AppLineageSection";

afterEach(() => {
  ENVELOPE = { blueprint: { seed: 0, steps: [{ kind: "model", model_id: "m", prompt: "hi" }] } };
});

describe("App Lineage (view-only)", () => {
  it("renders the read-only diagram + Run, with NO authoring controls (relocated to Workflows)", () => {
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByTestId("app-lineage")).toBeInTheDocument();
    expect(screen.getByTestId("lineage-readonly-notice")).toBeInTheDocument();
    expect(screen.getByTestId("app-lineage-run")).toBeInTheDocument();
    // Structure authoring lives in the Workflows builder now — none of it here.
    expect(screen.queryByTestId("app-lineage-save")).toBeNull();
    expect(screen.queryByTestId("lineage-add-agent")).toBeNull();
    expect(screen.queryByTestId("lineage-add-pure")).toBeNull();
    expect(screen.queryByTestId("lineage-add-tool")).toBeNull();
  });

  it("lays the parsed blueprint steps onto the diagram", () => {
    render(<AppLineageSection handle="apps/local/x" />);
    // The single model step from the seeded envelope renders as one diagram node.
    expect(screen.getByTestId("app-lineage-diagram")).toHaveAttribute("data-steps", "1");
  });

  it("an un-round-trippable (exec) blueprint still renders view-only (no authoring)", () => {
    ENVELOPE = {
      blueprint: { seed: 0, steps: [{ kind: "exec", body_signature_id: "a".repeat(64) }] },
    };
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByTestId("lineage-readonly-notice")).toBeInTheDocument();
    expect(screen.queryByTestId("app-lineage-save")).toBeNull();
  });
});
