/**
 * Security section: the single-user capability-manifest surface. An app picker (chips)
 * selects the App whose resolved warrant (reach / capability ceiling / model route) the
 * manifest panel renders. Honest empty / not-wired states; no cross-party RBAC.
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("../../src/components/apps/AppManifestPanel", () => ({
  AppManifestPanel: ({ handle }: { handle: string }) => (
    <div data-testid="manifest-panel">{handle}</div>
  ),
}));

let appsState: {
  apps: Array<{ handle: string; name: string }>;
  notWired: boolean;
  isLoading: boolean;
};
vi.mock("../../src/kx/use-apps", () => ({
  useApps: () => appsState,
}));

import { SystemsSection } from "../../src/components/sections/SystemsSection";

describe("SystemsSection (Security = capability manifest)", () => {
  it("keeps the Security heading + section handle", () => {
    appsState = { apps: [], notWired: false, isLoading: false };
    render(<SystemsSection />);
    expect(screen.getByTestId("systems-section")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Security" })).toBeInTheDocument();
  });

  it("shows the honest empty state when there are no apps", () => {
    appsState = { apps: [], notWired: false, isLoading: false };
    render(<SystemsSection />);
    expect(screen.getByText(/No apps yet/i)).toBeInTheDocument();
    expect(screen.queryByTestId("manifest-panel")).toBeNull();
  });

  it("defaults to the first app's manifest; a picker chip reports a new selection", () => {
    appsState = {
      apps: [
        { handle: "apps/local/echo", name: "Echo" },
        { handle: "apps/local/sum", name: "Summarize" },
      ],
      notWired: false,
      isLoading: false,
    };
    const onHandle = vi.fn();
    render(<SystemsSection onHandle={onHandle} />);
    // Default = the first app.
    expect(screen.getByTestId("manifest-panel")).toHaveTextContent("apps/local/echo");
    expect(screen.getByTestId("security-app-apps/local/echo")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    // Picking another chip reports the handle (the route drives the selection).
    fireEvent.click(screen.getByTestId("security-app-apps/local/sum"));
    expect(onHandle).toHaveBeenCalledWith("apps/local/sum");
  });

  it("honest-degrades when manifests are unsupported", () => {
    appsState = { apps: [], notWired: true, isLoading: false };
    render(<SystemsSection />);
    expect(screen.getByText(/need a newer gateway/i)).toBeInTheDocument();
  });
});
