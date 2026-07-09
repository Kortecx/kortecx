/**
 * POC-5c: the Apps "View" popup — a read-only envelope summary + project-branch
 * lineage snapshot (composes useApp + useBranches; no new RPC). These tests pin the
 * populated, no-branch (honest empty), and not-found states.
 */

import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const useAppMock = vi.fn();
const useBranchesMock = vi.fn();
const useAppManifestMock = vi.fn();

vi.mock("../../src/kx/use-apps", () => ({ useApp: (...a: unknown[]) => useAppMock(...a) }));
vi.mock("../../src/kx/use-branches", () => ({
  useBranches: (...a: unknown[]) => useBranchesMock(...a),
}));
vi.mock("../../src/kx/use-app-manifest", () => ({
  useAppManifest: (...a: unknown[]) => useAppManifestMock(...a),
}));

beforeEach(() => {
  // Default: no manifest (the section renders its honest empty state); overridden per test.
  useAppManifestMock.mockReturnValue({
    view: null,
    isLoading: false,
    notFound: false,
    error: null,
  });
});

import { AppViewPopover } from "../../src/components/apps/AppViewPopover";

function summary(over: Record<string, unknown> = {}) {
  return {
    handle: "kx/apps/demo",
    appRef: "aabbccdd11223344",
    name: "Demo App",
    version: "1",
    description: "A demo App",
    tags: ["agentic"],
    stepCount: 3,
    locked: false,
    ...over,
  };
}

function branch(over: Record<string, unknown> = {}) {
  return {
    handle: "kx/apps/demo",
    branchRef: "deadbeefdeadbeefdeadbeef",
    parentHandle: "",
    description: "",
    items: [],
    itemCount: 5,
    ...over,
  };
}

describe("AppViewPopover (POC-5c)", () => {
  it("renders the envelope summary + the project-branch lineage snapshot", () => {
    useAppMock.mockReturnValue({
      data: { summary: summary(), envelope: {} },
      isLoading: false,
      error: null,
    });
    useBranchesMock.mockReturnValue({ branches: [branch()], notWired: false });
    render(<AppViewPopover handle="kx/apps/demo" onClose={() => {}} />);

    expect(screen.getByTestId("app-view")).toBeInTheDocument();
    const facts = screen.getByTestId("app-view-summary");
    expect(facts).toHaveTextContent("Demo App");
    expect(facts).toHaveTextContent("v1");
    expect(facts).toHaveTextContent("agentic");
    // The lineage snapshot shows the real file count + short branch ref.
    expect(screen.getByTestId("app-view-branch")).toHaveTextContent("5");
  });

  it("shows an HONEST empty state when the App has no project branch yet (GR15)", () => {
    useAppMock.mockReturnValue({
      data: { summary: summary(), envelope: {} },
      isLoading: false,
      error: null,
    });
    useBranchesMock.mockReturnValue({ branches: [], notWired: false });
    render(<AppViewPopover handle="kx/apps/demo" onClose={() => {}} />);
    expect(screen.getByTestId("app-view-no-branch")).toHaveTextContent(/no project branch yet/i);
    expect(screen.queryByTestId("app-view-branch")).toBeNull();
  });

  it("renders a not-found state when the App is missing (no summary leaked)", () => {
    useAppMock.mockReturnValue({ data: null, isLoading: false, error: null });
    useBranchesMock.mockReturnValue({ branches: [], notWired: false });
    render(<AppViewPopover handle="kx/apps/missing" onClose={() => {}} />);
    expect(screen.getByText("Not found")).toBeInTheDocument();
    expect(screen.queryByTestId("app-view-summary")).toBeNull();
  });

  it("renders the capability manifest — needs vs. what you have", () => {
    useAppMock.mockReturnValue({
      data: { summary: summary(), envelope: {} },
      isLoading: false,
      error: null,
    });
    useBranchesMock.mockReturnValue({ branches: [], notWired: false });
    useAppManifestMock.mockReturnValue({
      view: {
        reachInherit: false,
        tools: [
          { id: "echo", version: "1", requested: true, inPolicy: true, inherited: false },
          { id: "gmail/search", version: "1", requested: true, inPolicy: false, inherited: false },
        ],
        connections: [
          {
            id: "mcp+stdio://gmail",
            version: "",
            requested: true,
            inPolicy: false,
            inherited: false,
          },
        ],
        modelRoute: "kx-serve:ghost",
        modelRouteServed: false,
        needsOnly: false,
      },
      isLoading: false,
      notFound: false,
      error: null,
    });
    render(<AppViewPopover handle="kx/apps/demo" onClose={() => {}} />);

    const tools = screen.getByTestId("app-manifest-tools");
    expect(tools).toHaveTextContent("echo@1");
    expect(tools).toHaveTextContent("satisfied");
    expect(tools).toHaveTextContent("gmail/search@1");
    expect(tools).toHaveTextContent("missing");
    // an unregistered connection shows as missing.
    expect(screen.getByTestId("app-manifest-connections")).toHaveTextContent("missing");
    // an unserved model route is flagged.
    expect(screen.getByTestId("app-manifest-model-status")).toHaveTextContent("not served");
  });
});
