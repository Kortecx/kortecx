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
let LOCKED = false;

vi.mock("../../src/kx/use-apps", () => ({
  useApp: () => ({
    data: { summary: { name: "X", locked: LOCKED }, envelope: ENVELOPE },
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
  LOCKED = false;
});

describe("App Lineage (view-only)", () => {
  it("renders the read-only diagram + Run, with NO inline authoring controls (relocated to Workflows)", () => {
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByTestId("app-lineage")).toBeInTheDocument();
    expect(screen.getByTestId("lineage-readonly-notice")).toBeInTheDocument();
    expect(screen.getByTestId("app-lineage-run")).toBeInTheDocument();
    // Inline structure authoring lives in the Workflows builder now — none of it here.
    expect(screen.queryByTestId("app-lineage-save")).toBeNull();
    expect(screen.queryByTestId("lineage-add-agent")).toBeNull();
    expect(screen.queryByTestId("lineage-add-pure")).toBeNull();
    expect(screen.queryByTestId("lineage-add-tool")).toBeNull();
  });

  it("offers an 'Edit structure' entry (to the builder) for a round-trippable, unlocked App", () => {
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByTestId("lineage-edit-structure")).toBeInTheDocument();
  });

  it("hides 'Edit structure' when the App is locked", () => {
    LOCKED = true;
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.queryByTestId("lineage-edit-structure")).toBeNull();
    // Run stays available on a locked App (a run is not a structure write).
    expect(screen.getByTestId("app-lineage-run")).toBeInTheDocument();
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
    // An un-round-trippable blueprint hides 'Edit structure' (never a lossy edit).
    expect(screen.queryByTestId("lineage-edit-structure")).toBeNull();
  });
});

/**
 * The granular per-step detail. The view-model's branches are exhausted in
 * `lineage-step-view.test.ts`; here the wiring is asserted — that what the model
 * derives actually reaches the card, keyed off the real envelope.
 */
describe("App Lineage (per-step detail)", () => {
  it("gives each step a DISTINCT title derived from its prompt (not N copies of 'Agent')", () => {
    ENVELOPE = {
      blueprint: {
        seed: 0,
        steps: [
          { kind: "model", model_id: "gemma-4-12b", prompt: "Research the target" },
          { kind: "model", model_id: "gemma-4-12b", prompt: "Draft the summary" },
        ],
        edges: [{ parent: 0, child: 1 }],
      },
    };
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByText("Research the target")).toBeInTheDocument();
    expect(screen.getByText("Draft the summary")).toBeInTheDocument();
  });

  it("shows each step's bound model, requested tools and authored budget", () => {
    ENVELOPE = {
      blueprint: {
        seed: 0,
        steps: [
          {
            kind: "model",
            model_id: "gemma-4-12b",
            prompt: "Search",
            tool_contract: { "web/search": "1" },
            max_turns: 8,
            max_tool_calls: 6,
          },
        ],
      },
    };
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByTestId("lineage-model-s0")).toHaveTextContent("gemma-4-12b");
    // "requests", never "has" — a tool_contract is a wish the server intersects (SN-8).
    expect(screen.getByTestId("lineage-tools-s0")).toHaveTextContent("requests");
    expect(screen.getByTestId("lineage-tools-s0")).toHaveTextContent("web/search");
    expect(screen.getByTestId("lineage-meta-s0")).toHaveTextContent("8 turns · 6 calls");
  });

  it("defers to the App's model_route when a step names no model of its own", () => {
    ENVELOPE = {
      blueprint: { seed: 0, steps: [{ kind: "model", prompt: "Go" }] },
      steering_config: { model: { model_route: "kx-serve:gemma" } },
    };
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByTestId("lineage-model-s0")).toHaveTextContent("inherits kx-serve:gemma");
  });

  it("degrades a bare pure step to its ordinal — no model, tools, or budget invented", () => {
    ENVELOPE = { blueprint: { seed: 0, steps: [{ kind: "pure" }] } };
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByText("Step 1")).toBeInTheDocument();
    expect(screen.queryByTestId("lineage-model-s0")).toBeNull();
    expect(screen.queryByTestId("lineage-tools-s0")).toBeNull();
    expect(screen.queryByTestId("lineage-meta-s0")).toBeNull();
  });

  it("renders NO budget when the blueprint omits it (the default is contested in-tree)", () => {
    ENVELOPE = {
      blueprint: {
        seed: 0,
        steps: [{ kind: "model", prompt: "Go", tool_contract: { "web/search": "1" } }],
      },
    };
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.queryByTestId("lineage-meta-s0")).toBeNull();
  });

  it("marks the entry step the server folds the App's skills + tool wishes onto", () => {
    ENVELOPE = {
      blueprint: {
        seed: 0,
        steps: [
          { kind: "model", prompt: "First" },
          { kind: "model", prompt: "Second" },
        ],
        edges: [{ parent: 0, child: 1 }],
      },
      references: { skills: [{ name: "summarize", instructions_ref: "a".repeat(64) }] },
    };
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByTestId("lineage-entry-s0")).toBeInTheDocument();
    expect(screen.queryByTestId("lineage-entry-s1")).toBeNull();
    expect(screen.queryByTestId("lineage-fold-warning")).toBeNull();
  });

  it("warns when attached skills have no root agent step to fold onto (the silent drop)", () => {
    // pure → model: the server refuses the split and drops the wishes with only a
    // server-side warning. The Skills rail would still show them as attached.
    ENVELOPE = {
      blueprint: {
        seed: 0,
        steps: [{ kind: "pure" }, { kind: "model", prompt: "Second" }],
        edges: [{ parent: 0, child: 1 }],
      },
      references: { skills: [{ name: "summarize", instructions_ref: "a".repeat(64) }] },
      steering_config: { tools: { requested_grants: { "web/search": "1" } } },
    };
    render(<AppLineageSection handle="apps/local/x" />);
    expect(screen.getByTestId("lineage-fold-warning")).toHaveTextContent(
      "2 skill/tool wishes can't be applied",
    );
    expect(screen.queryByTestId("lineage-entry-s0")).toBeNull();
    expect(screen.queryByTestId("lineage-entry-s1")).toBeNull();
  });
});
