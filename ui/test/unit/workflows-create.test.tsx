/**
 * The `/workflows/create` screen — the header form over the embedded builder:
 * the handle follows the name until touched, Save lowers the live graph into a
 * `kortecx.workflow/v1` envelope (draft toggle ⇒ caller-stated lifecycle) and
 * lands on the definition page, and `?handle=` seeds the form from the stored
 * envelope. Deliberately NO scaffold machinery — the save IS the authoring act.
 */

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render as rtlRender, screen, waitFor } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BuilderGraph } from "../../src/components/builder/builder-graph";

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

// A minimal VALID graph under the workflow (allowEmptyModel) rule — one served-model
// agent step, exactly what the real embedded builder starts with.
function starterGraph(): BuilderGraph {
  return {
    steps: [
      {
        id: "s0",
        kind: "model",
        label: "Agent",
        modelId: "",
        prompt: "summarize the overnight items",
        paramsText: "",
        reasoning: "",
        toolId: "",
        toolVersion: "",
        toolContract: {},
        skills: [],
        connections: [],
        datasets: [],
        apps: [],
        maxTurns: undefined,
        maxToolCalls: undefined,
      },
    ],
    edges: [],
  };
}

// The canvas is the builder suite's concern — stub it with a button that publishes
// the live graph the way embedded mode does (`onGraphChange`).
vi.mock("../../src/components/sections/BlueprintBuilderSection", () => ({
  BlueprintBuilderSection: ({
    onGraphChange,
  }: {
    onGraphChange?: (g: BuilderGraph) => void;
  }) => (
    <div data-testid="builder-stub">
      <button
        type="button"
        data-testid="stub-publish-graph"
        onClick={() => onGraphChange?.(starterGraph())}
      />
    </div>
  ),
}));

const saveMutate = vi.fn();
let SAVE_ERROR: unknown = null;
let STORED: {
  envelope: unknown;
  workflowDigest: string;
  lifecycle: string;
  stepCount: number;
} | null = null;
let STORED_LOADING = false;
vi.mock("../../src/kx/use-workflows", async () => {
  const actual = await vi.importActual<typeof import("../../src/kx/use-workflows")>(
    "../../src/kx/use-workflows",
  );
  return {
    WORKFLOW_SCHEMA: actual.WORKFLOW_SCHEMA,
    useSaveWorkflow: () => ({
      mutate: saveMutate,
      isPending: false,
      isError: SAVE_ERROR !== null,
      error: SAVE_ERROR,
      reset: vi.fn(),
    }),
    useWorkflow: () => ({
      data: STORED_LOADING ? undefined : STORED,
      isLoading: STORED_LOADING,
      isError: false,
      error: null,
      refetch: vi.fn(),
    }),
  };
});

import {
  CreateWorkflowScreen,
  defaultWorkflowHandle,
} from "../../src/components/sections/CreateWorkflowScreen";

afterEach(() => {
  navigateSpy.mockReset();
  saveMutate.mockReset();
  SAVE_ERROR = null;
  STORED = null;
  STORED_LOADING = false;
});

/** Mount fresh and publish the stub graph (the builder is lazy — wait for it). */
async function mountWithGraph(seedHandle: string | null = null) {
  render(<CreateWorkflowScreen seedHandle={seedHandle} />);
  await waitFor(() => expect(screen.getByTestId("builder-stub")).toBeInTheDocument());
  fireEvent.click(screen.getByTestId("stub-publish-graph"));
}

describe("defaultWorkflowHandle", () => {
  it("sanitizes into the workflows/local namespace", () => {
    expect(defaultWorkflowHandle("Morning Digest")).toBe("workflows/local/morning-digest");
    expect(defaultWorkflowHandle("")).toBe("workflows/local/workflow");
    expect(defaultWorkflowHandle("--x--")).toBe("workflows/local/x");
  });
});

describe("CreateWorkflowScreen", () => {
  it("derives the handle from the name until the user edits it", async () => {
    await mountWithGraph();
    fireEvent.change(screen.getByTestId("workflow-name"), {
      target: { value: "Morning Digest" },
    });
    expect(screen.getByTestId("workflow-handle")).toHaveValue("workflows/local/morning-digest");
    fireEvent.change(screen.getByTestId("workflow-handle"), {
      target: { value: "workflows/local/custom" },
    });
    fireEvent.change(screen.getByTestId("workflow-name"), { target: { value: "Renamed" } });
    // Touched ⇒ the user's word stands.
    expect(screen.getByTestId("workflow-handle")).toHaveValue("workflows/local/custom");
  });

  it("save is disabled until the workflow has a name", async () => {
    await mountWithGraph();
    expect(screen.getByTestId("workflow-save")).toBeDisabled();
    fireEvent.change(screen.getByTestId("workflow-name"), { target: { value: "Digest" } });
    expect(screen.getByTestId("workflow-save")).toBeEnabled();
  });

  it("save lowers the graph into a kortecx.workflow/v1 envelope and lands on the def page", async () => {
    saveMutate.mockImplementation(
      (_vars: unknown, opts?: { onSuccess?: (r: { handle: string }) => void }) =>
        opts?.onSuccess?.({ handle: "workflows/local/digest" }),
    );
    await mountWithGraph();
    fireEvent.change(screen.getByTestId("workflow-name"), { target: { value: "Digest" } });
    fireEvent.change(screen.getByTestId("workflow-description"), {
      target: { value: "The overnight summary" },
    });
    fireEvent.click(screen.getByTestId("workflow-save"));
    expect(saveMutate).toHaveBeenCalledTimes(1);
    const vars = saveMutate.mock.calls[0]?.[0] as {
      handle: string;
      envelope: Record<string, unknown>;
      lifecycle: string;
    };
    expect(vars.handle).toBe("workflows/local/digest");
    expect(vars.lifecycle).toBe("");
    expect(vars.envelope.schema).toBe("kortecx.workflow/v1");
    expect(vars.envelope.name).toBe("Digest");
    expect(vars.envelope.version).toBe("1");
    expect(vars.envelope.description).toBe("The overnight summary");
    const blueprint = vars.envelope.blueprint as { steps: unknown[] };
    expect(blueprint.steps).toHaveLength(1);
    expect(navigateSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        to: "/workflows/def/$handle",
        params: { handle: "workflows/local/digest" },
      }),
    );
  });

  it("an empty description is OMITTED (skip_serializing_if discipline, not an empty key)", async () => {
    await mountWithGraph();
    fireEvent.change(screen.getByTestId("workflow-name"), { target: { value: "Digest" } });
    fireEvent.click(screen.getByTestId("workflow-save"));
    const vars = saveMutate.mock.calls[0]?.[0] as { envelope: Record<string, unknown> };
    expect("description" in vars.envelope).toBe(false);
  });

  it("the draft toggle rides the save as the caller-stated lifecycle", async () => {
    await mountWithGraph();
    fireEvent.change(screen.getByTestId("workflow-name"), { target: { value: "Digest" } });
    fireEvent.click(screen.getByTestId("workflow-draft"));
    expect(screen.getByTestId("workflow-save").textContent).toContain("Save draft");
    fireEvent.click(screen.getByTestId("workflow-save"));
    expect(saveMutate.mock.calls[0]?.[0]).toEqual(expect.objectContaining({ lifecycle: "draft" }));
  });

  it("?handle= seeds the form + draft state from the stored envelope", async () => {
    STORED = {
      envelope: {
        schema: "kortecx.workflow/v1",
        name: "Morning digest",
        version: "1",
        description: "The overnight summary",
        blueprint: { seed: 0, steps: [{ kind: "model", prompt: "summarize" }] },
      },
      workflowDigest: "cd".repeat(32),
      lifecycle: "draft",
      stepCount: 1,
    };
    render(<CreateWorkflowScreen seedHandle="workflows/local/digest" />);
    expect(screen.getByTestId("workflow-name")).toHaveValue("Morning digest");
    expect(screen.getByTestId("workflow-description")).toHaveValue("The overnight summary");
    expect(screen.getByTestId("workflow-handle")).toHaveValue("workflows/local/digest");
    expect(screen.getByTestId("workflow-draft")).toBeChecked();
    expect(screen.getByText("Edit workflow")).toBeInTheDocument();
  });

  it("a seed handle with nothing stored says so and starts fresh", async () => {
    STORED = null;
    render(<CreateWorkflowScreen seedHandle="workflows/local/ghost" />);
    expect(screen.getByTestId("workflow-seed-missing")).toBeInTheDocument();
    expect(screen.getByTestId("workflow-name")).toHaveValue("");
  });

  it("shows the loading state while the seed is still being fetched", () => {
    STORED_LOADING = true;
    render(<CreateWorkflowScreen seedHandle="workflows/local/digest" />);
    expect(screen.getByText("Loading workflow…")).toBeInTheDocument();
    expect(screen.queryByTestId("workflow-name")).toBeNull();
  });
});
