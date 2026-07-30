/**
 * The workflow definition page (`/workflows/def/$handle`): the summary header
 * (name · description · steps · draft badge), the honest per-step list
 * ("requests" for tool wishes; a blank model says the server binds one), the
 * Run → scoped-run-view navigation, the draft ⇒ Finish-draft swap, the delete
 * confirm (safe-button focus, names what is kept), and the history drawer at
 * the workflow handle with blockedMessage null (nothing gates a workflow
 * restore client-side).
 */

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render as rtlRender, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

function render(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return rtlRender(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

const navigateSpy = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateSpy,
  Link: ({ children, to, params, search, ...rest }: Record<string, unknown>) => (
    <a {...(rest as Record<string, unknown>)}>{children as never}</a>
  ),
}));

interface Stored {
  envelope: unknown;
  workflowDigest: string;
  lifecycle: string;
  stepCount: number;
}
let STORED: Stored | null = null;
const runMutate = vi.fn();
const deleteMutate = vi.fn();
vi.mock("../../src/kx/use-workflows", () => ({
  useWorkflow: () => ({
    data: STORED,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useRunWorkflow: () => ({
    mutate: runMutate,
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
  useDeleteWorkflow: () => ({
    mutate: deleteMutate,
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
}));

// The generic drawer has its own suites (the App wrapper's + history-drawer.test);
// here we pin only WHAT the def page mounts it with.
const drawerProps = vi.fn();
vi.mock("../../src/components/HistoryDrawer", () => ({
  HistoryDrawer: (props: Record<string, unknown>) => {
    drawerProps(props);
    return <div data-testid="history-drawer-stub" />;
  },
}));

import { WorkflowDefSection } from "../../src/components/sections/WorkflowDefSection";

function stored(over: Partial<Stored> = {}): Stored {
  return {
    envelope: {
      schema: "kortecx.workflow/v1",
      name: "Morning digest",
      version: "1",
      description: "The overnight summary",
      blueprint: {
        seed: 0,
        steps: [
          { kind: "model", prompt: "summarize the overnight items" },
          { kind: "tool", tool_contract: { "kx.tool.retrieve": "1" } },
        ],
        edges: [{ parent: 0, child: 1 }],
      },
    },
    workflowDigest: "cd".repeat(32),
    lifecycle: "",
    stepCount: 2,
    ...over,
  };
}

afterEach(() => {
  STORED = null;
  navigateSpy.mockReset();
  runMutate.mockReset();
  deleteMutate.mockReset();
  drawerProps.mockReset();
});

describe("WorkflowDefSection", () => {
  it("renders the summary + the honest per-step list", () => {
    STORED = stored();
    render(<WorkflowDefSection handle="workflows/local/digest" />);
    expect(screen.getByTestId("workflow-def-name").textContent).toContain("Morning digest");
    expect(screen.getByTestId("workflow-def-description").textContent).toContain(
      "The overnight summary",
    );
    expect(screen.getByTestId("workflow-def-meta").textContent).toContain("2 steps");
    const step1 = screen.getByTestId("workflow-def-step-1");
    expect(step1.textContent).toContain("summarize the overnight items");
    // A model step naming no model is a run-time BINDING, said — never blanked.
    expect(step1.textContent).toContain("served model at run");
    // "requests", never "has": a tool_contract is a WISH.
    const step2 = screen.getByTestId("workflow-def-step-2");
    expect(step2.textContent).toContain("requests kx.tool.retrieve");
    expect(screen.queryByTestId("workflow-def-draft")).toBeNull();
  });

  it("Run fires the server-side run and navigates to the SCOPED run view", () => {
    runMutate.mockImplementation(
      (
        _vars: unknown,
        opts?: {
          onSuccess?: (r: {
            instanceId: string;
            reactChainSalt: string;
            terminalMoteId: string;
          }) => void;
        },
      ) =>
        opts?.onSuccess?.({
          instanceId: "ab".repeat(16),
          reactChainSalt: "",
          terminalMoteId: "ef".repeat(32),
        }),
    );
    STORED = stored();
    render(<WorkflowDefSection handle="workflows/local/digest" />);
    fireEvent.click(screen.getByTestId("workflow-def-run"));
    expect(runMutate.mock.calls[0]?.[0]).toEqual({ handle: "workflows/local/digest" });
    expect(navigateSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        to: "/workflows/$instanceId",
        params: { instanceId: "ab".repeat(16) },
        // No salt (a pure-DAG shape) ⇒ the anchor falls back to the terminal Mote.
        search: { terminal: "ef".repeat(32), chain: "ef".repeat(32) },
      }),
    );
  });

  it("a DRAFT swaps Run for Finish draft (→ the create screen seeded by ?handle=)", () => {
    STORED = stored({ lifecycle: "draft" });
    render(<WorkflowDefSection handle="workflows/local/digest" />);
    expect(screen.getByTestId("workflow-def-draft")).toBeInTheDocument();
    expect(screen.queryByTestId("workflow-def-run")).toBeNull();
    fireEvent.click(screen.getByTestId("workflow-def-finish"));
    expect(navigateSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        to: "/workflows/create",
        search: { handle: "workflows/local/digest" },
      }),
    );
  });

  it("History mounts the generic drawer at the workflow handle, ungated", () => {
    STORED = stored();
    render(<WorkflowDefSection handle="workflows/local/digest" />);
    fireEvent.click(screen.getByTestId("workflow-def-history"));
    expect(screen.getByTestId("history-drawer-stub")).toBeInTheDocument();
    expect(drawerProps).toHaveBeenCalledWith(
      expect.objectContaining({
        handle: "workflows/local/digest",
        blockedMessage: null,
        testIdPrefix: "workflow-history",
      }),
    );
  });

  it("Delete walks the confirm (Cancel focused) and the cascade copy names what stays", () => {
    STORED = stored();
    render(<WorkflowDefSection handle="workflows/local/digest" />);
    fireEvent.click(screen.getByTestId("workflow-def-delete"));
    const dialog = screen.getByTestId("workflow-delete-dialog");
    expect(dialog).toBeInTheDocument();
    // The SAFE button holds focus — a stray Enter must not delete.
    expect(screen.getByText("Cancel")).toHaveFocus();
    expect(screen.getByTestId("workflow-delete-kept").textContent).toContain("history");
    fireEvent.click(screen.getByTestId("workflow-delete-submit"));
    expect(deleteMutate.mock.calls[0]?.[0]).toEqual({ handle: "workflows/local/digest" });
  });

  it("not found is an honest empty state, not an error", () => {
    STORED = null;
    render(<WorkflowDefSection handle="workflows/local/ghost" />);
    expect(screen.getByText("Workflow not found")).toBeInTheDocument();
  });
});
