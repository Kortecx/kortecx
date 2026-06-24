/**
 * POC-4/POC-5a Apps section — the catalog plus the agentic "New App" entry. Apps
 * render as cards with Run · Open · Inspect; an honest empty state when none.
 * POC-5a flips the GR15 stance: a "New App" button NOW exists and toggles the
 * inline NewAppForm (the agentic scaffold). The catalog itself stays read-only.
 */

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render as rtlRender, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

// NewAppForm uses a raw react-query useMutation, so renders need a QueryClient.
function render(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return rtlRender(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

const mutate = vi.fn();
let APPS: Array<{
  handle: string;
  appRef: string;
  name: string;
  version: string;
  description: string;
  tags: string[];
  stepCount: number;
  locked: boolean;
}> = [];

vi.mock("../../src/kx/use-apps", () => ({
  useApps: () => ({
    apps: APPS,
    notWired: false,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useApp: () => ({ data: null, isLoading: false, error: null }),
  useRunApp: () => ({ mutate, isPending: false, error: null, reset: vi.fn() }),
}));

// NewAppForm dependencies — keep the form mountable + inert in jsdom.
vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({ client: null }),
}));
vi.mock("../../src/kx/use-scaffold-app", () => ({
  useScaffoldApp: () => ({ mutate: vi.fn(), isPending: false, error: null }),
  useScaffoldStatus: () => ({ data: undefined, isLoading: true, isError: false }),
  useInvalidateOnScaffoldDone: () => vi.fn(),
}));
vi.mock("@kortecx/sdk/web", () => ({
  minimalAppEnvelope: () => ({ schema: "kortecx.app/v1" }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));

import { AppsSection } from "../../src/components/sections/AppsSection";

afterEach(() => {
  APPS = [];
  mutate.mockReset();
});

describe("Apps section (catalog + POC-5a New App)", () => {
  it("renders saved Apps as cards with Run + Open + Inspect", () => {
    APPS = [
      {
        handle: "apps/local/echo",
        appRef: "ab".repeat(16),
        name: "Echo Demo",
        version: "1",
        description: "fires echo",
        tags: ["demo"],
        stepCount: 1,
        locked: false,
      },
    ];
    render(<AppsSection />);
    expect(screen.getByTestId("apps-section")).toBeInTheDocument();
    expect(screen.getByTestId("app-card-apps/local/echo")).toBeInTheDocument();
    expect(screen.getByTestId("app-run-apps/local/echo")).toBeInTheDocument();
    expect(screen.getByTestId("app-open-apps/local/echo")).toBeInTheDocument();
    expect(screen.getByTestId("app-inspect-apps/local/echo")).toBeInTheDocument();
  });

  it("POC-5a: the New App button EXISTS (flipped from POC-4)", () => {
    APPS = [];
    render(<AppsSection />);
    expect(screen.getByTestId("new-app")).toBeInTheDocument();
    // The form is collapsed until the button is clicked.
    expect(screen.queryByTestId("new-app-form")).toBeNull();
  });

  it("clicking New App reveals the inline scaffold form", () => {
    APPS = [];
    render(<AppsSection />);
    fireEvent.click(screen.getByTestId("new-app"));
    expect(screen.getByTestId("new-app-form")).toBeInTheDocument();
    expect(screen.getByTestId("new-app-name")).toBeInTheDocument();
    expect(screen.getByTestId("new-app-goal")).toBeInTheDocument();
    expect(screen.getByTestId("new-app-submit")).toBeInTheDocument();
  });

  it("shows a locked chip on a locked App", () => {
    APPS = [
      {
        handle: "apps/local/locked",
        appRef: "ef".repeat(16),
        name: "Locked App",
        version: "1",
        description: "",
        tags: [],
        stepCount: 1,
        locked: true,
      },
    ];
    render(<AppsSection />);
    expect(screen.getByTestId("app-card-locked-apps/local/locked")).toBeInTheDocument();
  });

  it("shows an honest empty state when the catalog is empty", () => {
    APPS = [];
    render(<AppsSection />);
    expect(screen.getByText(/no apps yet/i)).toBeInTheDocument();
  });

  it("Run fires runApp for the App's handle", () => {
    APPS = [
      {
        handle: "apps/local/pure",
        appRef: "cd".repeat(16),
        name: "Pure",
        version: "1",
        description: "",
        tags: [],
        stepCount: 1,
        locked: false,
      },
    ];
    render(<AppsSection />);
    screen.getByTestId("app-run-apps/local/pure").click();
    expect(mutate).toHaveBeenCalledWith(
      { handle: "apps/local/pure" },
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
  });
});
