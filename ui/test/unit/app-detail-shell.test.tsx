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
  useExportAppBundle: () => ({ mutate: vi.fn(), isPending: false, error: null, reset: vi.fn() }),
  useSaveApp: () => ({
    mutate: vi.fn(),
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
}));

// The header lock control's mutations are inert here (its own test drives lock/unlock);
// stub them so the shell test needs no live connection provider.
vi.mock("../../src/kx/use-app-lock", () => ({
  useLockApp: () => ({ mutate: vi.fn(), isPending: false, variables: undefined, error: null }),
  useUnlockApp: () => ({ mutate: vi.fn(), isPending: false, variables: undefined, error: null }),
}));

// The SkillsRail rides the detail shell; an empty catalog keeps these
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
  // The Files rail persists its collapsed flag to localStorage; reset between tests.
  localStorage.clear();
});

describe("App IDE shell (POC-5d)", () => {
  it("renders the tabs (Files / Lineage / Skills / Capabilities) + header controls", () => {
    render(<AppDetailSection handle="apps/local/echo" />);
    expect(screen.getByTestId("app-detail")).toBeInTheDocument();
    for (const t of ["files", "lineage", "skills", "capabilities"]) {
      expect(screen.getByTestId(`app-tab-${t}`)).toBeInTheDocument();
    }
    // Chat is a header action now (a right-side drawer), not a tab.
    expect(screen.queryByTestId("app-tab-chat")).toBeNull();
    expect(screen.getByTestId("app-detail-chat")).toBeInTheDocument();
    // The editable name shows the loaded envelope name; the lock control offers Lock.
    expect(screen.getByTestId("app-detail-name-input")).toHaveValue("Echo Demo");
    expect(screen.getByTestId("app-lock-apps/local/echo")).toBeInTheDocument();
  });

  it("the Chat header action opens the agentic Chat & Edit drawer (with the edit gate)", () => {
    render(<AppDetailSection handle="apps/local/echo" />);
    expect(screen.queryByTestId("app-chat-drawer")).toBeNull();
    fireEvent.click(screen.getByTestId("app-detail-chat"));
    expect(screen.getByTestId("app-chat-drawer")).toBeInTheDocument();
    // Unlocked ⇒ the propose→diff→approve edit affordance is present.
    expect(screen.getByTestId("app-chat-edit")).toBeInTheDocument();
    expect(screen.getByTestId("app-edit-propose")).toBeInTheDocument();
  });

  it("the Files rail is a collapsible sidebar (hides/shows the tree, persisted)", () => {
    render(<AppDetailSection handle="apps/local/echo" />);
    expect(screen.getByTestId("app-files-sidebar")).toBeInTheDocument();
    // The tree is expanded by default (the seeded file node renders).
    expect(screen.getByTestId("file-README.md")).toBeInTheDocument();
    // Collapsing hides the tree; expanding restores it.
    fireEvent.click(screen.getByTestId("app-files-collapse"));
    expect(screen.queryByTestId("file-README.md")).toBeNull();
    fireEvent.click(screen.getByTestId("app-files-collapse"));
    expect(screen.getByTestId("file-README.md")).toBeInTheDocument();
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

  it("LOCKED: shows the lock control + an honest notice, NO write affordances (GR15)", () => {
    LOCKED = true;
    render(<AppDetailSection handle="apps/local/echo" path="README.md" />);
    // The lock toggle reports locked (its Unlock affordance is the control); the name
    // input is disabled (a locked App can't be renamed — the server refuses the write).
    const lockToggle = screen.getByTestId("app-unlock-apps/local/echo");
    expect(lockToggle).toHaveAttribute("data-locked", "true");
    expect(screen.getByTestId("app-detail-name-input")).toBeDisabled();
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
