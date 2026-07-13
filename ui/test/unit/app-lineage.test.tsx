/**
 * POC-5d: the single-App Lineage editor. The real reactflow canvas is covered by the
 * browser E2E (jsdom can't measure a viewport); here `@xyflow/react` is stubbed so the
 * editor's logic is asserted: an editable App exposes Save, a LOCKED App hides Save +
 * shows the honest lock notice (GR15), and an un-round-trippable (exec) blueprint is
 * read-only.
 */

import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@xyflow/react", () => ({
  ReactFlow: ({ nodes }: { nodes: unknown[] }) => (
    <div data-testid="rf" data-nodes={nodes.length} />
  ),
  ReactFlowProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
  Background: () => null,
  Controls: () => null,
  MiniMap: () => null,
  addEdge: (_c: unknown, es: unknown[]) => es,
  useNodesState: (initial: unknown) => [initial, vi.fn(), vi.fn()],
  useEdgesState: (initial: unknown) => [initial, vi.fn(), vi.fn()],
  Handle: () => null,
  Position: { Top: "top", Bottom: "bottom" },
  MarkerType: { ArrowClosed: "arrowclosed" },
}));

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
  it("renders the read-only canvas + Run, with NO authoring controls (relocated to Workflows)", () => {
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

  it("lays the parsed blueprint steps onto the canvas", () => {
    render(<AppLineageSection handle="apps/local/x" />);
    // The single model step from the seeded envelope renders as one node.
    expect(screen.getByTestId("rf")).toHaveAttribute("data-nodes", "1");
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
