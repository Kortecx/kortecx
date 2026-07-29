/**
 * The App project-history drawer: versions render newest-first with cause chips,
 * the newest row is "current" (not restorable), restore walks a confirm dialog
 * whose copy states the append-only contract, a locked/scaffolding App refuses
 * up front with an honest notice, and an old gateway degrades to the not-wired
 * empty state instead of a broken list.
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
let NOT_WIRED = false;
const restoreMutate = vi.fn();

vi.mock("../../src/kx/use-branches", () => ({
  useBranchVersions: () => ({
    versions: VERSIONS,
    notWired: NOT_WIRED,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useRestoreBranch: () => ({
    mutate: restoreMutate,
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
  }),
}));
// The hosted restart affordance reaches useConnection (needs the provider);
// stub it — its own suite drives the hosted controls.
vi.mock("../../src/components/apps/HostedControls", () => ({
  HostedRestartButton: () => <button type="button" data-testid="hosted-restart-stub" />,
}));

import { AppHistoryDrawer } from "../../src/components/apps/AppHistoryDrawer";

function versions3(): FakeVersion[] {
  return [
    {
      version: 3,
      branchRef: "cc".repeat(16),
      recordedUnixMs: 1_700_000_200_000,
      cause: "advance",
      itemCount: 2,
    },
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
      itemCount: 0,
    },
  ];
}

function mount(over: Partial<Parameters<typeof AppHistoryDrawer>[0]> = {}) {
  return render(
    <AppHistoryDrawer
      handle="apps/local/demo"
      locked={false}
      scaffolding={false}
      hosted={false}
      onClose={vi.fn()}
      {...over}
    />,
  );
}

afterEach(() => {
  VERSIONS = null;
  NOT_WIRED = false;
  restoreMutate.mockReset();
});

describe("AppHistoryDrawer", () => {
  it("lists versions newest-first; the newest is current, older rows restore", () => {
    VERSIONS = versions3();
    mount();
    const rows = screen.getAllByTestId(/app-history-row-/);
    expect(rows.map((r) => r.getAttribute("data-testid"))).toEqual([
      "app-history-row-3",
      "app-history-row-2",
      "app-history-row-1",
    ]);
    expect(screen.getByTestId("app-history-current")).toBeInTheDocument();
    expect(screen.queryByTestId("app-history-restore-3")).toBeNull();
    expect(screen.getByTestId("app-history-restore-2")).toBeEnabled();
    expect(screen.getByText("create")).toBeInTheDocument();
  });

  it("restore walks the confirm dialog and fires the mutation with the version", () => {
    VERSIONS = versions3();
    mount();
    fireEvent.click(screen.getByTestId("app-history-restore-2"));
    const dialog = screen.getByTestId("app-history-confirm");
    expect(dialog.textContent).toContain("Nothing is deleted");
    fireEvent.click(screen.getByTestId("app-history-confirm-restore"));
    expect(restoreMutate).toHaveBeenCalledTimes(1);
    expect(restoreMutate.mock.calls[0]?.[0]).toEqual({ handle: "apps/local/demo", version: 2 });
  });

  it("a successful restore shows the recorded notice, and hosted adds the restart offer", () => {
    restoreMutate.mockImplementation((_vars: unknown, opts?: { onSuccess?: () => void }) =>
      opts?.onSuccess?.(),
    );
    VERSIONS = versions3();
    mount({ hosted: true });
    fireEvent.click(screen.getByTestId("app-history-restore-2"));
    // Hosted honesty in the confirm copy: the live server keeps serving old files.
    expect(screen.getByTestId("app-history-confirm").textContent).toContain("restart");
    fireEvent.click(screen.getByTestId("app-history-confirm-restore"));
    const notice = screen.getByTestId("app-history-restored");
    expect(notice.textContent).toContain("version 2");
    expect(screen.getByTestId("hosted-restart-stub")).toBeInTheDocument();
  });

  it("Escape closes the confirm first, then the drawer", () => {
    const onClose = vi.fn();
    VERSIONS = versions3();
    mount({ onClose });
    fireEvent.click(screen.getByTestId("app-history-restore-2"));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("app-history-confirm")).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("a locked App refuses up front: notice shown, restore disabled", () => {
    VERSIONS = versions3();
    mount({ locked: true });
    expect(screen.getByTestId("app-history-blocked").textContent).toContain("locked");
    expect(screen.getByTestId("app-history-restore-2")).toBeDisabled();
  });

  it("a live scaffold refuses up front with its own reason", () => {
    VERSIONS = versions3();
    mount({ scaffolding: true });
    expect(screen.getByTestId("app-history-blocked").textContent).toContain("scaffold");
  });

  it("an old gateway degrades to the not-wired empty state", () => {
    NOT_WIRED = true;
    mount();
    expect(screen.getByText("Project history needs a newer server")).toBeInTheDocument();
  });

  it("no recorded history is an honest empty state, not an error", () => {
    VERSIONS = null;
    mount();
    expect(screen.getByText("No recorded history yet")).toBeInTheDocument();
  });
});
