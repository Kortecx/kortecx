/**
 * POC-4 Apps section — the read-only catalog. Renders saved Apps as cards with
 * Run + Inspect actions; honest empty state when none. GR15: there is NO "New App"
 * / scaffold / create affordance here (authoring is the SDK/CLI; the agentic
 * scaffold lands in POC-5a) — a future re-skin that smuggles one in fails CI here.
 */

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const mutate = vi.fn();
let APPS: Array<{
  handle: string;
  appRef: string;
  name: string;
  version: string;
  description: string;
  tags: string[];
  stepCount: number;
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

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));

import { AppsSection } from "../../src/components/sections/AppsSection";

afterEach(() => {
  APPS = [];
  mutate.mockReset();
});

describe("Apps section (POC-4 read-only catalog)", () => {
  it("renders saved Apps as cards with Run + Inspect, never a create/New-App button", () => {
    APPS = [
      {
        handle: "apps/local/echo",
        appRef: "ab".repeat(16),
        name: "Echo Demo",
        version: "1",
        description: "fires echo",
        tags: ["demo"],
        stepCount: 1,
      },
    ];
    render(<AppsSection />);
    expect(screen.getByTestId("apps-section")).toBeInTheDocument();
    expect(screen.getByTestId("app-card-apps/local/echo")).toBeInTheDocument();
    expect(screen.getByTestId("app-run-apps/local/echo")).toBeInTheDocument();
    expect(screen.getByTestId("app-inspect-apps/local/echo")).toBeInTheDocument();
    // GR15: authoring is the SDK/CLI; no scaffold/New-App button in POC-4.
    expect(screen.queryByRole("button", { name: /new app|create|scaffold/i })).toBeNull();
    expect(screen.queryByTestId("new-app")).toBeNull();
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
