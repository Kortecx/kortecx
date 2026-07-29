/**
 * The `/apps/create` screen — the whole create journey to a TERMINAL result:
 * compose (the NewAppForm pins that used to live in apps-section.test.tsx) →
 * launch → live scaffold → the create-result dialog (success: usable app with
 * Open/Done; failure: error + draft story with Resume/Discard/Close).
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

// The scaffold status the screen polls once a launch succeeded; tests flip it.
let PHASE: "planning" | "writing" | "done" | "failed" | null = null;
let DETAIL = "";
const resumeMutate = vi.fn();
vi.mock("../../src/kx/use-scaffold-app", () => ({
  useScaffoldApp: () => ({
    mutate: resumeMutate,
    isPending: false,
    error: null,
    reset: vi.fn(),
  }),
  useScaffoldStatus: () => ({
    data: PHASE === null ? undefined : { phase: PHASE, detail: DETAIL },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useInvalidateOnScaffoldDone: () => vi.fn(),
}));
const deleteMutate = vi.fn();
vi.mock("../../src/kx/use-apps", () => ({
  useDeleteApp: () => ({ mutate: deleteMutate, isPending: false, error: null, reset: vi.fn() }),
  // The REAL NewAppForm (rendered by the compose-pin suite below) reads the
  // catalog for its handle-collision check.
  useApps: () => ({
    apps: [],
    notWired: false,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
}));
// The form and the live scaffold view have their own suites — stub both so this
// one isolates the screen's state machine (launch outcome → terminal dialog).
vi.mock("../../src/components/sections/NewAppForm", () => ({
  NewAppForm: ({
    onLaunched,
  }: {
    onLaunched: (o: { handle: string; kind: string; launchError: string | null }) => void;
  }) => (
    <div data-testid="new-app-form-stub">
      <button
        type="button"
        data-testid="stub-launch-ok"
        onClick={() => onLaunched({ handle: "apps/local/x", kind: "scheduled", launchError: null })}
      />
      <button
        type="button"
        data-testid="stub-launch-fail"
        onClick={() =>
          onLaunched({ handle: "apps/local/x", kind: "scheduled", launchError: "no served model" })
        }
      />
      <button
        type="button"
        data-testid="stub-launch-hosted"
        onClick={() => onLaunched({ handle: "apps/local/h", kind: "hosted", launchError: null })}
      />
    </div>
  ),
}));
vi.mock("../../src/components/sections/ScaffoldProgress", () => ({
  ScaffoldProgress: () => <div data-testid="scaffold-progress-stub" />,
}));

import { CreateAppScreen } from "../../src/components/sections/CreateAppScreen";

afterEach(() => {
  PHASE = null;
  DETAIL = "";
  navigateSpy.mockReset();
  resumeMutate.mockReset();
  deleteMutate.mockReset();
});

describe("CreateAppScreen", () => {
  it("shows the form until launch, then the live scaffold in its place", () => {
    render(<CreateAppScreen />);
    expect(screen.getByTestId("new-app-form-stub")).toBeInTheDocument();
    PHASE = "writing";
    fireEvent.click(screen.getByTestId("stub-launch-ok"));
    expect(screen.queryByTestId("new-app-form-stub")).toBeNull();
    expect(screen.getByTestId("scaffold-progress-stub")).toBeInTheDocument();
    expect(screen.queryByTestId("app-create-result")).toBeNull();
  });

  it("done ⇒ the success dialog: Open goes to the app, Done goes back to Apps", () => {
    render(<CreateAppScreen />);
    PHASE = "done";
    fireEvent.click(screen.getByTestId("stub-launch-ok"));
    const dialog = screen.getByTestId("app-create-result");
    expect(dialog).toHaveAttribute("data-outcome", "done");
    fireEvent.click(screen.getByTestId("app-create-result-open"));
    expect(navigateSpy).toHaveBeenCalledWith(
      expect.objectContaining({ to: "/apps/$handle", params: { handle: "apps/local/x" } }),
    );
    fireEvent.click(screen.getByTestId("app-create-result-done"));
    expect(navigateSpy).toHaveBeenCalledWith(expect.objectContaining({ to: "/apps" }));
  });

  it("a scaffold that FAILS shows the error + the draft story with Resume/Discard/Close", () => {
    render(<CreateAppScreen />);
    PHASE = "failed";
    DETAIL = "step timed out";
    fireEvent.click(screen.getByTestId("stub-launch-ok"));
    const dialog = screen.getByTestId("app-create-result");
    expect(dialog).toHaveAttribute("data-outcome", "failed");
    expect(screen.getByTestId("app-create-result-detail").textContent).toContain("step timed out");
    expect(dialog.textContent).toContain("draft");
    fireEvent.click(screen.getByTestId("app-create-result-resume"));
    expect(resumeMutate).toHaveBeenCalledWith(
      { handle: "apps/local/x" },
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
  });

  it("a launch FAILURE is a terminal result immediately — no scaffold to watch", () => {
    render(<CreateAppScreen />);
    fireEvent.click(screen.getByTestId("stub-launch-fail"));
    const dialog = screen.getByTestId("app-create-result");
    expect(dialog).toHaveAttribute("data-outcome", "failed");
    expect(screen.getByTestId("app-create-result-detail").textContent).toContain("no served model");
    expect(screen.queryByTestId("scaffold-progress-stub")).toBeNull();
  });

  it("Discard deletes the draft and returns to Apps", () => {
    render(<CreateAppScreen />);
    fireEvent.click(screen.getByTestId("stub-launch-fail"));
    fireEvent.click(screen.getByTestId("app-create-result-discard"));
    expect(deleteMutate).toHaveBeenCalledWith(
      { handle: "apps/local/x" },
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
  });

  it("a HOSTED app's Done routes home to the Hosted section (the kind-follow)", () => {
    render(<CreateAppScreen />);
    PHASE = "done";
    fireEvent.click(screen.getByTestId("stub-launch-hosted"));
    fireEvent.click(screen.getByTestId("app-create-result-done"));
    expect(navigateSpy).toHaveBeenCalledWith(
      expect.objectContaining({ to: "/apps", search: { section: "hosted" } }),
    );
  });

  it("Resume's success re-arms the scaffold view (back to watching)", () => {
    resumeMutate.mockImplementation((_vars: unknown, opts?: { onSuccess?: () => void }) =>
      opts?.onSuccess?.(),
    );
    render(<CreateAppScreen />);
    fireEvent.click(screen.getByTestId("stub-launch-fail"));
    fireEvent.click(screen.getByTestId("app-create-result-resume"));
    // The launch error cleared ⇒ the live scaffold view replaces the failure note.
    expect(screen.getByTestId("scaffold-progress-stub")).toBeInTheDocument();
    expect(screen.queryByTestId("apps-create-launch-failed")).toBeNull();
  });

  it("Escape on a launch-failure result closes it and returns home", () => {
    render(<CreateAppScreen />);
    fireEvent.click(screen.getByTestId("stub-launch-fail"));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(navigateSpy).toHaveBeenCalledWith(expect.objectContaining({ to: "/apps" }));
  });
});

// ---- the compose-surface pins, moved from apps-section.test.tsx -------------
// These render the REAL NewAppForm (its deps no-op under a null client).

vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({ client: null, endpoint: "e", status: "connected" }),
}));
vi.mock("@kortecx/sdk/web", () => ({
  minimalAppEnvelope: () => ({ schema: "kortecx.app/v1" }),
}));

describe("the compose surface (real NewAppForm)", () => {
  it("shows ONE prompt box with the selectors on it, and no name field yet", async () => {
    const { NewAppForm: RealForm } = await vi.importActual<
      typeof import("../../src/components/sections/NewAppForm")
    >("../../src/components/sections/NewAppForm");
    render(<RealForm onClose={vi.fn()} onLaunched={vi.fn()} />);
    expect(screen.getByTestId("new-app-form")).toBeInTheDocument();
    expect(screen.getByTestId("new-app-prompt")).toBeInTheDocument();
    expect(screen.getByTestId("new-app-kind")).toBeInTheDocument();
    expect(screen.getByTestId("new-app-mode")).toBeInTheDocument();
    expect(screen.getByTestId("new-app-derive")).toBeInTheDocument();
    expect(screen.queryByTestId("new-app-name")).toBeNull();
  });

  it("the design button stays disabled until there is a prompt", async () => {
    const { NewAppForm: RealForm } = await vi.importActual<
      typeof import("../../src/components/sections/NewAppForm")
    >("../../src/components/sections/NewAppForm");
    render(<RealForm onClose={vi.fn()} onLaunched={vi.fn()} />);
    expect(screen.getByTestId("new-app-derive")).toBeDisabled();
    fireEvent.change(screen.getByTestId("new-app-prompt"), { target: { value: "triage email" } });
    expect(screen.getByTestId("new-app-derive")).not.toBeDisabled();
  });
});
