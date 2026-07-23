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
/** The App lane the mocked `useApp` reports: "" ⇒ functional/scheduled. */
let KIND = "";
const saveFile = vi.fn();
const proposeMutate = vi.fn();

vi.mock("../../src/kx/use-apps", () => ({
  useApp: () => ({
    data: {
      summary: { name: "Echo Demo", locked: LOCKED, kind: KIND },
      envelope: { input_schema: null },
    },
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
// The per-App trigger strip + its ScheduleButton reach useConnection, which THROWS
// outside KxConnectionProvider — this suite wraps in QueryClientProvider only. An empty,
// wired registry keeps the strip mounted (so the shell can assert it) and inert.
vi.mock("../../src/kx/use-triggers", () => ({
  useListTriggers: () => ({
    triggers: [],
    notWired: false,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useRegisterTrigger: () => ({
    mutate: vi.fn(),
    isPending: false,
    isError: false,
    isSuccess: false,
    error: null,
  }),
  useDeregisterTrigger: () => ({
    mutate: vi.fn(),
    isPending: false,
    variables: undefined,
    error: null,
  }),
  useTestTrigger: () => ({ mutate: vi.fn(), isPending: false, data: undefined, error: null }),
  useFireTrigger: () => ({ mutate: vi.fn(), isPending: false, data: undefined, error: null }),
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
// The Files tab asks whether a scaffold is WRITING this App right now (the chat surface routes
// here the moment an App is created). `done` ⇒ the tab shows the file tree, which is what every
// assertion below is about.
vi.mock("../../src/kx/use-scaffold-app", () => ({
  useScaffoldStatus: () => ({ data: { phase: "done" } }),
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
// The hosted cluster reaches useConnection, which THROWS outside KxConnectionProvider —
// and this suite wraps in QueryClientProvider only. Stub it so the SCHEDULED cases stay
// unaffected and the hosted case can assert which controls are offered.
vi.mock("../../src/components/apps/HostedControls", () => ({
  HostedStatusPill: () => <div data-testid="hosted-pill-stub" />,
  HostedRunButton: () => <div data-testid="hosted-run-stub" />,
  HostedStopButton: () => <div data-testid="hosted-stop-stub" />,
  HostedRestartButton: () => <div data-testid="hosted-restart-stub" />,
}));
vi.mock("../../src/components/apps/HostedRunPanel", () => ({
  HostedRunPanel: () => <div data-testid="hosted-panel-stub" />,
}));

import { AppDetailSection } from "../../src/components/sections/AppDetailSection";

function render(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return rtlRender(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

afterEach(() => {
  LOCKED = false;
  KIND = "";
  saveFile.mockReset();
  proposeMutate.mockReset();
  // The Files rail persists its collapsed flag to localStorage; reset between tests.
  localStorage.clear();
});

describe("App IDE shell (POC-5d)", () => {
  it("renders the tabs (Files / Lineage / Skills / MCP Tools / Integrations) + header controls", () => {
    render(<AppDetailSection handle="apps/local/echo" />);
    expect(screen.getByTestId("app-detail")).toBeInTheDocument();
    for (const t of ["files", "lineage", "skills", "tools", "integrations"]) {
      expect(screen.getByTestId(`app-tab-${t}`)).toBeInTheDocument();
    }
    // The old single "Capabilities" tab is gone (split into MCP Tools + Integrations).
    expect(screen.queryByTestId("app-tab-capabilities")).toBeNull();
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

  // D213: the page had NO kind check, so it offered a hosted App the scheduled-lane Run
  // (which RunApp refuses for having no blueprint) and none of the lifecycle controls
  // the catalog already had.
  it("a HOSTED app gets lifecycle controls, not the blueprint Run", () => {
    KIND = "experience";
    render(<AppDetailSection handle="apps/local/site" />);
    expect(screen.queryByTestId("app-detail-run")).toBeNull();
    expect(screen.getByTestId("hosted-pill-stub")).toBeInTheDocument();
    expect(screen.getByTestId("hosted-run-stub")).toBeInTheDocument();
    expect(screen.getByTestId("hosted-stop-stub")).toBeInTheDocument();
    expect(screen.getByTestId("hosted-restart-stub")).toBeInTheDocument();
    // Modify/Download/Lock are lane-independent and stay.
    expect(screen.getByTestId("app-detail-chat")).toBeInTheDocument();
    expect(screen.getByTestId("app-detail-download")).toBeInTheDocument();
  });

  it("a HOSTED app's Lineage tab shows the server panel, not an empty blueprint", () => {
    KIND = "experience";
    render(<AppDetailSection handle="apps/local/site" />);
    fireEvent.click(screen.getByTestId("app-tab-lineage"));
    expect(screen.getByTestId("hosted-panel-stub")).toBeInTheDocument();
    // The blueprint diagram — which would render "No steps" plus an Edit-structure
    // affordance the server cannot honour — is not mounted at all.
    expect(screen.queryByTestId("app-lineage-stub")).toBeNull();
  });

  // The tab strip had NO kind filter, so a hosted App offered Skills / MCP Tools /
  // Integrations — three rails the hosted lane provably never reads (hostsupervisor.rs
  // launches from the `hosted` config alone; the code that resolves a tool wish, a skill
  // and a connector lives behind RunApp, which refuses a hosted App).
  it("a HOSTED app offers only the tabs its lane reads (no Skills / Tools / Integrations)", () => {
    KIND = "experience";
    render(<AppDetailSection handle="apps/local/site" />);
    for (const t of ["files", "lineage"]) {
      expect(screen.getByTestId(`app-tab-${t}`)).toBeInTheDocument();
    }
    for (const t of ["skills", "tools", "integrations"]) {
      expect(screen.queryByTestId(`app-tab-${t}`)).toBeNull();
    }
  });

  it("a HOSTED app deep-linked to a lane-less tab falls back to Files", () => {
    KIND = "experience";
    render(<AppDetailSection handle="apps/local/site" tab="skills" />);
    // No skills rail, and the Files pane is what rendered instead.
    expect(screen.queryByTestId("app-skills-rail")).toBeNull();
    expect(screen.getByTestId("app-files-sidebar")).toBeInTheDocument();
  });

  // ScheduleButton accepted an `appHandle` target and had zero call sites; the strip is
  // where a scheduled App's schedule now lives. A hosted App gets neither (a trigger
  // fires through RunApp, which refuses it).
  it("a SCHEDULED app carries the per-App trigger strip; a hosted one does not", () => {
    const { unmount } = render(<AppDetailSection handle="apps/local/echo" />);
    expect(screen.getByTestId("app-triggers-strip")).toBeInTheDocument();
    expect(screen.getByTestId("app-schedule-apps/local/echo")).toBeInTheDocument();
    unmount();
    KIND = "experience";
    render(<AppDetailSection handle="apps/local/site" />);
    expect(screen.queryByTestId("app-triggers-strip")).toBeNull();
  });
});
