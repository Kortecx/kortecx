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
// ONE stable spy, not a fresh `vi.fn()` per call. The old mock handed back a new
// function each render, so nothing could assert where Run actually navigated — which is
// why this surface shipped sending a salt-only search that was empty for almost every App.
const navigateSpy = vi.fn();
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigateSpy }));
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
  navigateSpy.mockReset();
});

/** Fire Run, then hand the mutation the RunHandle a server would have returned. */
function runAndSettle(started: Record<string, unknown>): void {
  fireEvent.click(screen.getByTestId("app-run-now"));
  const call = runMutate.mock.calls[0];
  if (!call) {
    throw new Error("Run now did not call runApp");
  }
  (call[1] as { onSuccess: (r: unknown) => void }).onSuccess(started);
}

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

  // ▶ on an App is the most-travelled path to a run view, and it was landing UNSCOPED for
  // almost every App: it sent `{ chain: reactChainSalt }` only, and the server emits that
  // salt exclusively for a run with exactly one tool-granted agentic step. An ordinary
  // scheduled App is not that shape, so the condition was false and the view fell back to
  // the whole shared journal.
  it("scopes the run view by the terminal Mote when there is no agentic salt", () => {
    render(<AppRunDrawer handle="apps/local/echo" onClose={vi.fn()} />);
    runAndSettle({
      instanceId: "ab".repeat(8),
      reactChainSalt: "",
      terminalMoteId: "cd".repeat(32),
    });

    expect(navigateSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        params: { instanceId: "ab".repeat(8) },
        // `terminal` is the poll-stop hint, `chain` the scope anchor. Both are the sink
        // Mote here, and they are separate keys because they answer separate questions.
        search: { terminal: "cd".repeat(32), chain: "cd".repeat(32) },
      }),
    );
  });

  it("prefers the agentic chain salt when the run has one", () => {
    // Both anchors present: the salt wins, because the react surfaces thread it through as
    // `step_salt` and it pins the agentic chain exactly.
    render(<AppRunDrawer handle="apps/local/echo" onClose={vi.fn()} />);
    runAndSettle({
      instanceId: "ab".repeat(8),
      reactChainSalt: "ef".repeat(32),
      terminalMoteId: "cd".repeat(32),
    });

    expect(navigateSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        // The salt anchors the SCOPE; the terminal still rides along as the poll-stop
        // hint. Conflating the two is what the separate keys exist to prevent.
        search: { terminal: "cd".repeat(32), chain: "ef".repeat(32) },
      }),
    );
  });

  it("sends no anchor when the server gave neither, rather than inventing one", () => {
    // An old server. The run view must show its honest "every step in this journal"
    // notice; fabricating an anchor to silence that would restore the original bug.
    render(<AppRunDrawer handle="apps/local/echo" onClose={vi.fn()} />);
    runAndSettle({ instanceId: "ab".repeat(8), reactChainSalt: "", terminalMoteId: "" });

    expect(navigateSpy).toHaveBeenCalledWith(expect.objectContaining({ search: {} }));
  });

  it("with input_schema: renders the typed form (no bare Run now)", () => {
    INPUT_SCHEMA = { fields: [{ name: "word", type: "str", required: true }] };
    render(<AppRunDrawer handle="apps/local/echo" onClose={vi.fn()} />);
    expect(screen.queryByTestId("app-run-now")).toBeNull();
    // the recipe-form renders an input for the field
    expect(screen.getByLabelText(/word/i)).toBeInTheDocument();
  });
});
