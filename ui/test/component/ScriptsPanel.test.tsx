/** ScriptsPanel — the script registry govern surface: the not-wired / loading /
 *  error / empty / list states, the register form, and the per-row deregister.
 *  The kx hooks are mocked → a pure render/interaction check.
 *
 *  The central assertion is a wording one, and it is load-bearing rather than
 *  cosmetic: a row states what a script REQUESTED, never what it may do. The
 *  declared wish is only a request — the runtime refuses any call whose
 *  requirement the caller does not already hold — so a panel labelling those
 *  fields "granted" would tell an operator the opposite of the truth. */

import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const listState = {
  scripts: [] as Array<Record<string, unknown>>,
  hasMore: false,
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
const registerM = mut(vi.fn());
const deregisterM = mut(vi.fn());

vi.mock("../../src/kx/use-scripts", () => ({
  useListScripts: () => listState,
  useRegisterScript: () => registerM,
  useDeregisterScript: () => deregisterM,
}));

import { ScriptsPanel } from "../../src/components/tools/ScriptsPanel";

function script(over: Record<string, unknown> = {}) {
  return {
    scriptId: "aa".repeat(8),
    scriptName: "report/summarize",
    scriptVersion: "1",
    interpreter: "python3",
    description: "Summarise the weekly report.",
    sourceRef: "beef".repeat(16),
    fsScope: "ro:/srv/data",
    netScope: "none",
    wallClockMs: 30_000,
    maxOutputBytes: 1_048_576,
    ...over,
  };
}

function resetMut(m: ReturnType<typeof mut>) {
  m.isPending = false;
  m.variables = undefined;
  m.error = null;
  m.mutate.mockClear();
}

afterEach(() => {
  listState.scripts = [];
  listState.notWired = false;
  listState.isLoading = false;
  listState.isError = false;
  listState.error = null;
  resetMut(registerM);
  resetMut(deregisterM);
});

describe("ScriptsPanel", () => {
  it("degrades to an honest not-wired state on an older gateway", () => {
    listState.notWired = true;
    render(<ScriptsPanel />);
    expect(screen.getByText(/Scripts need a newer gateway/)).toBeInTheDocument();
    // Not an empty registry — saying "no scripts" for a gateway that cannot
    // answer would be a fabricated fact about the server.
    expect(screen.queryByText(/No scripts registered/)).toBeNull();
  });

  it("shows a loading state, then an empty one", () => {
    listState.isLoading = true;
    const { rerender } = render(<ScriptsPanel />);
    expect(screen.getByText(/Loading scripts/)).toBeInTheDocument();

    listState.isLoading = false;
    rerender(<ScriptsPanel />);
    expect(screen.getByText(/No scripts registered/)).toBeInTheDocument();
  });

  it("replaces the panel with an error notice, offering a retry", () => {
    listState.isError = true;
    listState.error = new Error("boom");
    render(<ScriptsPanel />);
    // The registry body is gone — a half-rendered list beside an error would
    // read as "these are the scripts" when the list is not known.
    expect(screen.queryByTestId("scripts-panel")).toBeNull();
    const retry = screen.getByRole("button", { name: /retry/i });
    fireEvent.click(retry);
    expect(listState.refetch).toHaveBeenCalled();
  });

  it("lists a registered script with what it REQUESTED, not what it holds", () => {
    listState.scripts = [script()];
    render(<ScriptsPanel />);

    expect(screen.getByTestId("registered-script-report/summarize-1")).toHaveTextContent(
      "report/summarize@1",
    );
    // The wording that matters: requested, never granted.
    expect(screen.getByText("files requested")).toBeInTheDocument();
    expect(screen.getByText("network requested")).toBeInTheDocument();
    expect(screen.queryByText(/granted/i)).toBeNull();
    expect(screen.getByText("ro:/srv/data")).toBeInTheDocument();

    // The row shows the source's ref so an operator can tell two registrations
    // of the same name apart.
    expect(screen.getByTitle("beef".repeat(16))).toBeInTheDocument();
  });

  it("deregisters by exact name and version", () => {
    listState.scripts = [script()];
    render(<ScriptsPanel />);
    fireEvent.click(screen.getByTestId("deregister-script-report/summarize-1"));
    expect(deregisterM.mutate).toHaveBeenCalledWith({
      name: "report/summarize",
      version: "1",
    });
  });

  it("registers a script, parsing the mount lines and dropping malformed ones", () => {
    render(<ScriptsPanel />);
    fireEvent.change(screen.getByTestId("script-name"), {
      target: { value: "ops/rollup" },
    });
    fireEvent.change(screen.getByTestId("script-interpreter"), {
      target: { value: "node" },
    });
    fireEvent.change(screen.getByTestId("script-source"), {
      target: { value: "console.log('hi')" },
    });
    fireEvent.change(screen.getByTestId("script-mounts"), {
      // The middle line has no recognised mode and must be dropped rather than
      // sent as something the server will reject with a less obvious message.
      target: { value: "ro:/srv/in\nnonsense\nrw:/srv/out\n" },
    });
    fireEvent.submit(screen.getByTestId("register-script-form"));

    expect(registerM.mutate).toHaveBeenCalledTimes(1);
    const [input] = registerM.mutate.mock.calls[0] as [Record<string, unknown>];
    expect(input.name).toBe("ops/rollup");
    expect(input.interpreter).toBe("node");
    expect(input.fsMounts).toEqual([
      { mode: "ro", path: "/srv/in" },
      { mode: "rw", path: "/srv/out" },
    ]);
  });

  it("keeps a path containing a colon intact", () => {
    render(<ScriptsPanel />);
    fireEvent.change(screen.getByTestId("script-name"), {
      target: { value: "ops/x" },
    });
    fireEvent.change(screen.getByTestId("script-source"), {
      target: { value: "printf x" },
    });
    fireEvent.change(screen.getByTestId("script-mounts"), {
      target: { value: "ro:/srv/a:b" },
    });
    fireEvent.submit(screen.getByTestId("register-script-form"));
    const [input] = registerM.mutate.mock.calls[0] as [Record<string, unknown>];
    // Splitting on every colon would truncate this to "/srv/a".
    expect(input.fsMounts).toEqual([{ mode: "ro", path: "/srv/a:b" }]);
  });

  it("shows a register failure inline", () => {
    registerM.error = new Error("no usable python3 on this host");
    render(<ScriptsPanel />);
    expect(screen.getByTestId("register-script-error")).toHaveTextContent(/no usable python3/);
  });
});
