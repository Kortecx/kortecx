/** PR-6b-4 AutoGrantStatus — the honest auto-grant status row. `useRecipes` is
 *  mocked so the test is a pure render check: ON iff `kx/recipes/react-auto` is
 *  in the recipe catalog, OFF (the default) otherwise. */

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const recipesState = {
  data: undefined as string[] | undefined,
};

vi.mock("../../src/kx/use-recipes", () => ({
  useRecipes: () => recipesState,
}));

import { AutoGrantStatus } from "../../src/components/tools/AutoGrantStatus";

afterEach(() => {
  recipesState.data = undefined;
});

describe("AutoGrantStatus", () => {
  it("reads OFF when react-auto is absent (the default-OFF posture)", () => {
    recipesState.data = ["kx/recipes/echo", "kx/recipes/react"];
    render(<AutoGrantStatus />);
    expect(screen.getByTestId("autogrant-pill")).toHaveTextContent("OFF");
  });

  it("reads OFF while the catalog is loading / unwired", () => {
    recipesState.data = undefined;
    render(<AutoGrantStatus />);
    expect(screen.getByTestId("autogrant-pill")).toHaveTextContent("OFF");
  });

  it("reads ON when react-auto is provisioned", () => {
    recipesState.data = ["kx/recipes/react", "kx/recipes/react-auto"];
    render(<AutoGrantStatus />);
    expect(screen.getByTestId("autogrant-pill")).toHaveTextContent("ON");
  });
});
