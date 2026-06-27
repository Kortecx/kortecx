/** SecretsPanel (MM-3 / D110) — the host secret-store govern surface: the not-wired
 *  / loading / empty / list states, the add form (name + write-only value), and the
 *  per-row remove. The kx hooks are mocked → a pure render/interaction check. The
 *  central assertion: a secret VALUE is NEVER displayed back (D81). */

import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const listState = {
  names: [] as Array<Record<string, unknown>>,
  notWired: false,
  isLoading: false,
  isError: false,
  error: null as unknown,
  refetch: vi.fn(),
};
const mut = (mutate: ReturnType<typeof vi.fn>) => ({
  mutate,
  isPending: false,
  variables: undefined as unknown,
  error: null as unknown,
  isSuccess: false,
  data: undefined as unknown,
});
const putM = mut(vi.fn());
const removeM = mut(vi.fn());

vi.mock("../../src/kx/use-secrets", () => ({
  useListSecretNames: () => listState,
  usePutSecret: () => putM,
  useDeleteSecret: () => removeM,
}));

import { SecretsPanel } from "../../src/components/tools/SecretsPanel";

function resetMut(m: ReturnType<typeof mut>) {
  m.isPending = false;
  m.variables = undefined;
  m.error = null;
  m.isSuccess = false;
  m.data = undefined;
  m.mutate.mockClear();
}

afterEach(() => {
  listState.names = [];
  listState.notWired = false;
  listState.isLoading = false;
  [putM, removeM].forEach(resetMut);
});

describe("SecretsPanel", () => {
  it("shows the honest not-wired empty state", () => {
    listState.notWired = true;
    render(<SecretsPanel />);
    expect(screen.getByText("Secret store not enabled")).toBeInTheDocument();
  });

  it("shows the loading state", () => {
    listState.isLoading = true;
    render(<SecretsPanel />);
    expect(screen.getByText("Loading secrets…")).toBeInTheDocument();
  });

  it("shows the empty state + the add form when no secrets are stored", () => {
    render(<SecretsPanel />);
    expect(screen.getByText("No secrets stored")).toBeInTheDocument();
    expect(screen.getByTestId("secret-add-form")).toBeInTheDocument();
  });

  it("renders a stored secret NAME + remove, and NEVER displays a value (D81)", () => {
    listState.names = [
      { name: "github_token", createdUnixMs: 1_700_000_000_000, updatedUnixMs: 0 },
    ];
    render(<SecretsPanel />);
    expect(screen.getByTestId("secret-github_token")).toBeInTheDocument();
    expect(screen.getByText("github_token")).toBeInTheDocument();
    // No secret value is ever in the document — only the NAME + the write-only chip.
    expect(screen.getByText("write-only")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("secret-remove-github_token"));
    expect(removeM.mutate).toHaveBeenCalledWith("github_token");
  });

  it("the value input is type=password and write-only (never echoed)", () => {
    render(<SecretsPanel />);
    const value = screen.getByTestId("secret-add-value") as HTMLInputElement;
    expect(value.type).toBe("password");
    // The store mutation carries the value once; the panel never reads it back.
    fireEvent.change(screen.getByTestId("secret-add-name"), { target: { value: "api_key" } });
    fireEvent.change(value, { target: { value: "s3cr3t" } });
    fireEvent.submit(screen.getByTestId("secret-add-form"));
    expect(putM.mutate).toHaveBeenCalledTimes(1);
    const args = putM.mutate.mock.calls[0]?.[0];
    expect(args).toMatchObject({ name: "api_key", value: "s3cr3t" });
    // The plaintext value is not rendered anywhere as visible text.
    expect(screen.queryByText("s3cr3t")).toBeNull();
  });

  it("does not submit without both a name and a value", () => {
    render(<SecretsPanel />);
    fireEvent.change(screen.getByTestId("secret-add-name"), { target: { value: "api_key" } });
    fireEvent.submit(screen.getByTestId("secret-add-form"));
    expect(putM.mutate).not.toHaveBeenCalled();
  });
});
