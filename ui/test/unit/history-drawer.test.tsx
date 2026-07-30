/**
 * The GENERIC history drawer's parameterization (the App wrapper's behavior is
 * pinned by app-history-drawer.test.tsx, unchanged): the testid prefix scopes
 * every hook point, cause labels/titles are caller-overridable per entity, an
 * ungated drawer (blockedMessage null) restores freely, and the entity-specific
 * `invalidateOnRestore` keys reach `useRestoreBranch` (the appBranch trap —
 * without this the workflow tree never refreshes after a restore).
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

interface FakeVersion {
  version: number;
  branchRef: string;
  recordedUnixMs: number;
  cause: string;
  itemCount: number;
}

let VERSIONS: FakeVersion[] | null = null;
const restoreMutate = vi.fn();
const restoreArgs = vi.fn();

vi.mock("../../src/kx/use-branches", () => ({
  useBranchVersions: () => ({
    versions: VERSIONS,
    notWired: false,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useRestoreBranch: (opts?: unknown) => {
    restoreArgs(opts);
    return {
      mutate: restoreMutate,
      isPending: false,
      isError: false,
      error: null,
      reset: vi.fn(),
    };
  },
}));

import { HistoryDrawer } from "../../src/components/HistoryDrawer";

function versions2(): FakeVersion[] {
  return [
    {
      version: 2,
      branchRef: "bb".repeat(16),
      recordedUnixMs: 1_700_000_100_000,
      cause: "advance",
      itemCount: 1,
    },
    {
      version: 1,
      branchRef: "aa".repeat(16),
      recordedUnixMs: 1_700_000_000_000,
      cause: "create",
      itemCount: 1,
    },
  ];
}

function mount(over: Partial<Parameters<typeof HistoryDrawer>[0]> = {}) {
  return render(
    <HistoryDrawer
      handle="workflows/local/digest"
      title="Definition history"
      blockedMessage={null}
      emptyState={{ title: "No recorded history yet", detail: "Every save records here." }}
      testIdPrefix="workflow-history"
      onClose={vi.fn()}
      {...over}
    />,
  );
}

afterEach(() => {
  VERSIONS = null;
  restoreMutate.mockReset();
  restoreArgs.mockReset();
});

describe("HistoryDrawer (generic)", () => {
  it("scopes every testid by the caller's prefix", () => {
    VERSIONS = versions2();
    mount();
    expect(screen.getByTestId("workflow-history")).toBeInTheDocument();
    expect(screen.getByTestId("workflow-history-list")).toBeInTheDocument();
    expect(screen.getByTestId("workflow-history-row-2")).toBeInTheDocument();
    expect(screen.getByTestId("workflow-history-current")).toBeInTheDocument();
    expect(screen.getByTestId("workflow-history-restore-1")).toBeEnabled();
  });

  it("cause labels are caller-overridable (an entity says 'Saved', not 'Edited')", () => {
    VERSIONS = versions2();
    mount({ causeLabels: { advance: "Saved" } });
    expect(screen.getByText("Saved")).toBeInTheDocument();
    expect(screen.queryByText("Edited")).toBeNull();
    // Unoverridden causes keep the default reader's word.
    expect(screen.getByText("Created")).toBeInTheDocument();
  });

  it("blockedMessage gates up front; null leaves restore free", () => {
    VERSIONS = versions2();
    const { unmount } = mount({ blockedMessage: "This entity is busy — restore is refused." });
    expect(screen.getByTestId("workflow-history-blocked").textContent).toContain("busy");
    expect(screen.getByTestId("workflow-history-restore-1")).toBeDisabled();
    unmount();
    mount();
    expect(screen.queryByTestId("workflow-history-blocked")).toBeNull();
    expect(screen.getByTestId("workflow-history-restore-1")).toBeEnabled();
  });

  it("plumbs invalidateOnRestore into useRestoreBranch and fires the restore", () => {
    VERSIONS = versions2();
    const invalidate = (endpoint: string, handle: string) => [["kx", endpoint, "workflow", handle]];
    mount({ invalidateOnRestore: invalidate, confirmTitle: "Restore this workflow?" });
    expect(restoreArgs).toHaveBeenCalledWith({ invalidate });
    fireEvent.click(screen.getByTestId("workflow-history-restore-1"));
    expect(screen.getByTestId("workflow-history-confirm").textContent).toContain(
      "Restore this workflow?",
    );
    fireEvent.click(screen.getByTestId("workflow-history-confirm-restore"));
    expect(restoreMutate.mock.calls[0]?.[0]).toEqual({
      handle: "workflows/local/digest",
      version: 1,
    });
  });
});
