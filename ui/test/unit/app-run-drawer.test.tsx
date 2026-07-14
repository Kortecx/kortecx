/**
 * POC-5d: the single-App run drawer. An App with NO input_schema runs in one click;
 * an App WITH inputs renders the typed RecipeForm (no bare "Run now").
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

let INPUT_SCHEMA: unknown = null;
const runMutate = vi.fn();

vi.mock("../../src/kx/use-apps", () => ({
  useApp: () => ({ data: { envelope: { input_schema: INPUT_SCHEMA } }, isLoading: false }),
  useRunApp: () => ({
    mutate: runMutate,
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
// The Run preflight (advisory feasibility) reads the manifest + served models.
vi.mock("../../src/kx/use-app-manifest", () => ({
  useAppManifest: () => ({ view: null, notFound: false, isLoading: false, error: null }),
}));
vi.mock("../../src/kx/use-models", () => ({
  useModels: () => ({ models: [], unsupported: false, loading: false }),
}));

import { AppRunDrawer } from "../../src/components/apps/AppRunDrawer";

afterEach(() => {
  INPUT_SCHEMA = null;
  runMutate.mockReset();
});

describe("App run drawer (POC-5d)", () => {
  it("no inputs: a single Run now button fires runApp with empty args", () => {
    render(<AppRunDrawer handle="apps/local/echo" onClose={vi.fn()} />);
    expect(screen.getByTestId("app-run-drawer")).toBeInTheDocument();
    const run = screen.getByTestId("app-run-now");
    fireEvent.click(run);
    expect(runMutate).toHaveBeenCalledWith(
      { handle: "apps/local/echo", args: {} },
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
  });

  it("with input_schema: renders the typed form (no bare Run now)", () => {
    INPUT_SCHEMA = { fields: [{ name: "word", type: "str", required: true }] };
    render(<AppRunDrawer handle="apps/local/echo" onClose={vi.fn()} />);
    expect(screen.queryByTestId("app-run-now")).toBeNull();
    // the recipe-form renders an input for the field
    expect(screen.getByLabelText(/word/i)).toBeInTheDocument();
  });
});
