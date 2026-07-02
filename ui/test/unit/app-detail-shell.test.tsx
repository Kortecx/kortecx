/**
 * POC-5d: the single-App IDE shell — the full-screen tabbed workspace. Asserts the
 * 3 tabs render, a LOCKED App disables every write affordance with an honest notice
 * (GR15 absence guard), and an unlocked file exposes both direct + agentic edit.
 * Sub-sections (Lineage / Chat / Run drawer) are stubbed so this test isolates the
 * shell + the Files pane.
 */

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render as rtlRender, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

let LOCKED = false;
const saveFile = vi.fn();
const proposeMutate = vi.fn();

vi.mock("../../src/kx/use-apps", () => ({
  useApp: () => ({
    data: { summary: { name: "Echo Demo", locked: LOCKED }, envelope: { input_schema: null } },
    isLoading: false,
    isError: false,
    error: null,
  }),
  useRunApp: () => ({ mutate: vi.fn(), isPending: false, error: null, reset: vi.fn() }),
  useSaveApp: () => ({
    mutate: vi.fn(),
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
}));

// RC-SW1: the SkillsRail rides the detail shell; an empty catalog keeps these
// shell tests focused on the IDE affordances.
vi.mock("../../src/kx/use-skills", () => ({
  useListSkills: () => ({
    skills: [],
    notWired: false,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
}));
vi.mock("../../src/kx/use-app-files", () => ({
  useAppBranch: () => ({
    data: { items: [{ path: "README.md", contentRef: "ab".repeat(32) }] },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useAppFileContent: () => ({
    data: { text: "# hello\nworld", missing: false },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useSaveFile: () => ({
    mutate: saveFile,
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
}));
vi.mock("../../src/kx/use-branches", () => ({
  useEditBranchPropose: () => ({
    mutate: proposeMutate,
    isPending: false,
    isError: false,
    error: null,
    data: null,
    reset: vi.fn(),
  }),
  useAdvanceBranch: () => ({
    mutate: vi.fn(),
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));

// Isolate the shell — stub the heavy sub-sections (their own tests cover them).
vi.mock("../../src/components/sections/AppLineageSection", () => ({
  AppLineageSection: () => <div data-testid="app-lineage-stub" />,
}));
vi.mock("../../src/components/apps/AppRunDrawer", () => ({
  AppRunDrawer: () => <div data-testid="app-run-drawer-stub" />,
}));
vi.mock("../../src/components/chat/AppChat", () => ({
  AppChat: () => <div data-testid="app-chat-stub" />,
}));

import { AppDetailSection } from "../../src/components/sections/AppDetailSection";

function render(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return rtlRender(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

afterEach(() => {
  LOCKED = false;
  saveFile.mockReset();
  proposeMutate.mockReset();
});

describe("App IDE shell (POC-5d)", () => {
  it("renders the 3 tabs (Files / Lineage / Chat)", () => {
    render(<AppDetailSection handle="apps/local/echo" />);
    expect(screen.getByTestId("app-detail")).toBeInTheDocument();
    expect(screen.getByTestId("app-tab-files")).toBeInTheDocument();
    expect(screen.getByTestId("app-tab-lineage")).toBeInTheDocument();
    expect(screen.getByTestId("app-tab-chat")).toBeInTheDocument();
  });

  it("unlocked: a selected file exposes direct + agentic edit", () => {
    render(<AppDetailSection handle="apps/local/echo" path="README.md" />);
    expect(screen.getByTestId("app-file-edit-direct")).toBeInTheDocument();
    expect(screen.getByTestId("app-file-edit-agentic")).toBeInTheDocument();
    // Direct edit → Monaco editor + Save appear.
    fireEvent.click(screen.getByTestId("app-file-edit-direct"));
    expect(screen.getByTestId("app-file-direct-editor")).toBeInTheDocument();
    expect(screen.getByTestId("app-file-save")).toBeInTheDocument();
  });

  it("LOCKED: shows the lock chip + an honest notice, NO write affordances (GR15)", () => {
    LOCKED = true;
    render(<AppDetailSection handle="apps/local/echo" path="README.md" />);
    expect(screen.getByTestId("app-detail-locked")).toBeInTheDocument();
    expect(screen.getByTestId("app-locked-notice")).toBeInTheDocument();
    // The runtime refuses writes; the UI must never offer a control that can't fire.
    expect(screen.queryByTestId("app-file-edit-direct")).toBeNull();
    expect(screen.queryByTestId("app-file-edit-agentic")).toBeNull();
    expect(screen.queryByTestId("app-file-save")).toBeNull();
  });

  it("Run opens the run drawer", () => {
    render(<AppDetailSection handle="apps/local/echo" />);
    expect(screen.queryByTestId("app-run-drawer-stub")).toBeNull();
    fireEvent.click(screen.getByTestId("app-detail-run"));
    expect(screen.getByTestId("app-run-drawer-stub")).toBeInTheDocument();
  });
});
