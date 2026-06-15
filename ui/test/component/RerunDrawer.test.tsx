import { RecipeForm as RecipeFormDef, RecipeFormField } from "@kortecx/sdk/web";
import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RunRecord } from "../../src/lib/recent-runs";

// RerunDrawer pulls in the router + four kx hooks; stub them so the drawer renders
// without a provider and we can drive each branch (local vs durable inputs, pure
// vs world-mutating projection, no-change vs fresh invoke result).
const h = vi.hoisted(() => ({
  navigate: vi.fn(),
  onSuccessResult: null as {
    instanceId: string;
    terminalMoteId: string;
    recipeFingerprint: string;
  } | null,
  invokeMutate: vi.fn(),
  invokeState: { isPending: false, error: null as unknown },
  runInputs: { data: undefined as unknown, isLoading: false, isError: false },
  projection: { data: undefined as unknown, isSuccess: false },
  form: { data: undefined as unknown, isLoading: false, error: null as unknown, refetch: vi.fn() },
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children, ...rest }: any) =>
    React.createElement("a", { href: typeof to === "string" ? to : "#", ...rest }, children),
  useNavigate: () => h.navigate,
}));
vi.mock("../../src/kx/use-invoke", () => ({
  useInvoke: () => ({
    mutate: h.invokeMutate,
    isPending: h.invokeState.isPending,
    error: h.invokeState.error,
  }),
}));
vi.mock("../../src/kx/use-run-inputs", () => ({ useRunInputs: () => h.runInputs }));
vi.mock("../../src/kx/use-projection", () => ({ useProjection: () => h.projection }));
vi.mock("../../src/kx/use-recipes", () => ({ useRecipeForm: () => h.form }));

import { RerunDrawer } from "../../src/components/sections/RerunDrawer";

const ECHO_FORM = new RecipeFormDef("kx/recipes/echo", [
  new RecipeFormField("topic", "str", true, 4096, []),
]);

function localRun(over: Partial<RunRecord> = {}): RunRecord {
  return {
    instanceId: "ab".repeat(16),
    terminalMoteId: "cd".repeat(32),
    recipeFingerprint: null,
    handle: "kx/recipes/echo",
    startedAt: 0,
    args: JSON.stringify({ topic: "hi" }),
    ...over,
  };
}

beforeEach(() => {
  h.navigate.mockReset();
  h.invokeMutate.mockReset();
  h.invokeMutate.mockImplementation((_vars, opts?: { onSuccess?: (r: unknown) => void }) => {
    if (h.onSuccessResult && opts?.onSuccess) {
      opts.onSuccess(h.onSuccessResult);
    }
  });
  h.onSuccessResult = null;
  h.invokeState = { isPending: false, error: null };
  h.runInputs = { data: undefined, isLoading: false, isError: false };
  h.projection = { data: { motes: [{ ndClass: 1 }] }, isSuccess: true }; // pure by default
  h.form = { data: ECHO_FORM, isLoading: false, error: null, refetch: vi.fn() };
});

describe("RerunDrawer", () => {
  it("pre-fills the form with the run's local args", () => {
    render(<RerunDrawer run={localRun()} onClose={vi.fn()} />);
    expect(screen.getByTestId("rerun-drawer")).toBeInTheDocument();
    expect(screen.getByTestId("field-topic")).toHaveValue("hi");
  });

  it("a pure run re-runs WITHOUT a confirm; an unchanged result shows the no-change banner", () => {
    const run = localRun();
    // The kernel dedups identical args back to the SAME terminal mote.
    h.onSuccessResult = {
      instanceId: run.instanceId,
      terminalMoteId: run.terminalMoteId as string,
      recipeFingerprint: "",
    };
    render(<RerunDrawer run={run} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /run blueprint/i }));
    expect(h.invokeMutate).toHaveBeenCalledTimes(1); // no confirm gate for a pure run
    expect(screen.getByTestId("rerun-no-change")).toBeInTheDocument();
    expect(h.navigate).not.toHaveBeenCalled();
  });

  it("a changed result navigates to the new run (no banner)", () => {
    const run = localRun();
    h.onSuccessResult = {
      instanceId: run.instanceId,
      terminalMoteId: "ef".repeat(32), // a different terminal ⇒ fresh sub-DAG
      recipeFingerprint: "",
    };
    render(<RerunDrawer run={run} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /run blueprint/i }));
    expect(h.navigate).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId("rerun-no-change")).not.toBeInTheDocument();
  });

  it("a WORLD_MUTATING prior run gates the re-run behind a confirm", () => {
    h.projection = { data: { motes: [{ ndClass: 3 }] }, isSuccess: true };
    render(<RerunDrawer run={localRun()} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /run blueprint/i }));
    // Confirm panel appears; the invoke has NOT fired yet.
    const confirm = screen.getByTestId("rerun-confirm");
    expect(confirm).toHaveTextContent(/world-mutating/i);
    expect(h.invokeMutate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("rerun-confirm-fire"));
    expect(h.invokeMutate).toHaveBeenCalledTimes(1);
  });

  it("a durable run with no captured inputs degrades honestly (no form)", () => {
    h.runInputs = { data: undefined, isLoading: false, isError: true };
    render(<RerunDrawer run={localRun({ handle: null, args: null })} onClose={vi.fn()} />);
    expect(screen.getByText(/inputs not captured/i)).toBeInTheDocument();
    expect(screen.queryByTestId("recipe-form")).not.toBeInTheDocument();
  });
});
