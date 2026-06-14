/** PR-4.1b RunCard — clean headline, the per-card action menu (Cloud-disabled
 *  Share/Schedule, NO settings on an immutable run), export wiring + rename. */

import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RunRecord } from "../../src/lib/recent-runs";

// TanStack Links → plain anchors (router integration is covered by the e2e).
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, params, activeProps, children, ...rest }: any) =>
    React.createElement("a", { href: typeof to === "string" ? to : "#", ...rest }, children),
  useNavigate: () => vi.fn(),
}));
vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({ endpoint: "http://ep" }),
}));
vi.mock("../../src/kx/use-invoke", () => ({
  useInvoke: () => ({ mutate: vi.fn(), isPending: false, error: null }),
}));

const exportLight = vi.fn();
const exportRich = vi.fn(() => Promise.resolve());
vi.mock("../../src/kx/use-run-export", () => ({
  useRunExport: () => ({ exportLight, exportRich, pendingId: null, error: null }),
}));

import { RunCard } from "../../src/components/sections/RunCard";

const base: RunRecord = {
  instanceId: "ab".repeat(16),
  terminalMoteId: "cd".repeat(16),
  recipeFingerprint: "ef".repeat(16),
  handle: "kx/recipes/echo",
  startedAt: 1_700_000_000_000,
  args: '{"topic":"hi"}',
};

afterEach(() => {
  exportLight.mockClear();
  exportRich.mockClear();
  localStorage.clear();
});

describe("RunCard", () => {
  it("shows the clean headline + the raw handle as a secondary chip", () => {
    render(<RunCard run={base} headline="Echo" rawHandle="kx/recipes/echo" customName={null} />);
    expect(screen.getByTestId("run-open")).toHaveTextContent("Echo");
    expect(screen.getByText("kx/recipes/echo")).toBeInTheDocument();
  });

  it("opens the action menu; Share/Schedule are Cloud-disabled and there is NO settings item", () => {
    render(<RunCard run={base} headline="Echo" rawHandle="kx/recipes/echo" customName={null} />);
    fireEvent.click(screen.getByTestId("run-menu"));
    // The real, enabled actions.
    expect(screen.getByTestId("run-open-newtab")).toHaveAttribute("target", "_blank");
    expect(screen.getByTestId("run-open-newtab")).toHaveAttribute("rel", "noopener noreferrer");
    expect(screen.getByTestId("run-clone")).toBeInTheDocument();
    expect(screen.getByTestId("run-remix")).toBeInTheDocument();
    expect(screen.getByTestId("run-export-rich")).toBeInTheDocument(); // has a terminal Mote
    // Honest-disabled cloud capabilities.
    for (const id of ["run-share", "run-schedule"]) {
      const item = screen.getByTestId(id);
      expect(item).toBeDisabled();
      expect(item).toHaveTextContent("Cloud");
    }
    // An immutable run has no "settings" affordance (don't-fake-gaps).
    expect(screen.queryByTestId("run-settings")).toBeNull();
  });

  it("exports the run record (lightweight) and the rich bundle", () => {
    render(<RunCard run={base} headline="Echo" rawHandle="kx/recipes/echo" customName={null} />);
    fireEvent.click(screen.getByTestId("run-menu"));
    fireEvent.click(screen.getByTestId("run-export"));
    expect(exportLight).toHaveBeenCalledWith(base, "Echo");
    fireEvent.click(screen.getByTestId("run-menu"));
    fireEvent.click(screen.getByTestId("run-export-rich"));
    expect(exportRich).toHaveBeenCalledWith(base, "Echo");
  });

  it("rename reveals the inline input and persists to the client-local store", () => {
    render(<RunCard run={base} headline="Echo" rawHandle="kx/recipes/echo" customName={null} />);
    fireEvent.click(screen.getByTestId("run-menu"));
    fireEvent.click(screen.getByTestId("run-rename"));
    const input = screen.getByTestId("run-rename-input");
    fireEvent.change(input, { target: { value: "incident triage" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(localStorage.getItem("kortecx.ui.run-names:http://ep")).toContain("incident triage");
  });

  it("hides handle-only actions for a durable journal-only run", () => {
    const journalOnly: RunRecord = {
      instanceId: "11".repeat(16),
      terminalMoteId: null,
      recipeFingerprint: "ef".repeat(16),
      handle: null,
      startedAt: 1_700_000_000_000,
      args: null,
    };
    render(<RunCard run={journalOnly} headline="Echo" rawHandle={null} customName={null} />);
    fireEvent.click(screen.getByTestId("run-menu"));
    expect(screen.queryByTestId("run-again")).toBeNull();
    expect(screen.queryByTestId("run-clone")).toBeNull();
    expect(screen.queryByTestId("run-export-rich")).toBeNull(); // no terminal Mote
    expect(screen.getByText("journal")).toBeInTheDocument();
  });
});
