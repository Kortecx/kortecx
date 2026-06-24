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

describe("App Lineage editor (POC-5d)", () => {
  it("editable App: renders the canvas + Save-to-App + add controls", () => {
    render(<AppLineageSection handle="apps/local/x" locked={false} />);
    expect(screen.getByTestId("app-lineage")).toBeInTheDocument();
    expect(screen.getByTestId("app-lineage-save")).toBeInTheDocument();
    expect(screen.getByTestId("lineage-add-agent")).toBeInTheDocument();
    expect(screen.queryByTestId("lineage-locked-notice")).toBeNull();
  });

  it("LOCKED App: no Save, an honest lock notice (GR15)", () => {
    render(<AppLineageSection handle="apps/local/x" locked={true} />);
    expect(screen.getByTestId("lineage-locked-notice")).toBeInTheDocument();
    expect(screen.queryByTestId("app-lineage-save")).toBeNull();
    expect(screen.queryByTestId("lineage-add-agent")).toBeNull();
  });

  it("served-model agentic App (empty modelId): Save is enabled, not blocked on a model", () => {
    // The portable App convention — an agentic step binds the SERVED model (no
    // model_id). The lineage editor must NOT flag "needs a model" (it would block
    // every real served-model App from re-saving). Regression for the live-review find.
    ENVELOPE = {
      blueprint: {
        seed: 0,
        steps: [
          {
            kind: "model",
            prompt: "Use the echo tool.",
            tool_contract: { "mcp-echo/echo": "1" },
            params: { max_turns: "4", max_tool_calls: "2" },
          },
        ],
      },
    };
    render(<AppLineageSection handle="apps/local/x" locked={false} />);
    const save = screen.getByTestId("app-lineage-save");
    expect(save).toBeInTheDocument();
    expect(save).not.toBeDisabled();
    expect(screen.queryByTestId("lineage-validation")).toBeNull();
  });

  it("un-round-trippable (exec) blueprint: read-only, no Save", () => {
    ENVELOPE = {
      blueprint: { seed: 0, steps: [{ kind: "exec", body_signature_id: "a".repeat(64) }] },
    };
    render(<AppLineageSection handle="apps/local/x" locked={false} />);
    expect(screen.getByTestId("lineage-readonly-notice")).toBeInTheDocument();
    expect(screen.queryByTestId("app-lineage-save")).toBeNull();
  });
});
