import {
  AssetGrants,
  GrantView,
  KxNotFound,
  KxUnimplemented,
  TeamMember,
  TeamMembers,
  TeamSummary,
  WarrantView,
} from "@kortecx/sdk/web";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { SystemsSection } from "../../src/components/sections/SystemsSection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const demoTeam = new TeamSummary("kx/teams/demo", "Demo Team", "kx-gateway", 3);
const members = new TeamMembers("kx-gateway", [
  new TeamMember(
    "alice@acme",
    "demo-delegate",
    ["Read", "Use", "Delegate"],
    new WarrantView("Bwrap", "m ×3 (4096/512 tok)", "None", "/tmp/in:ReadOnly", 3, 1000, 30000),
  ),
  new TeamMember("bob@acme", "demo-member", ["Read", "Use"], null),
]);
const echoGrants = new AssetGrants("kx-gateway", [
  new GrantView("kx-gateway", "kx/teams/demo", ["Read", "Use"], "demo", true, false),
  new GrantView("kx-gateway", "alice@acme", ["Read", "Use"], "demo", true, false),
]);

function fullMock() {
  return makeMockClient({
    listSignatures: async () => [],
    listTeams: async () => [demoTeam],
    listTeamMembers: async () => members,
    listRecipes: async () => ["kx/recipes/echo"],
    listAssetGrants: async () => echoGrants,
  });
}

describe("SystemsSection", () => {
  it("renders the team, its members, and a delegate badge", async () => {
    const { client } = fullMock();
    render(<SystemsSection />, { wrapper: connectedWrapper(client) });

    expect(screen.getByTestId("teams-panel")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("team-pick-kx/teams/demo")).toBeInTheDocument());
    await waitFor(() => expect(screen.getByTestId("member-table")).toBeInTheDocument());
    expect(screen.getByTestId("member-row-alice@acme")).toBeInTheDocument();
    // alice is a delegate → the delegate badge.
    const delegates = document.querySelectorAll(".role-badge--delegate");
    expect(delegates).toHaveLength(1);
  });

  it("shows the grants inspector with the team grant on echo", async () => {
    const { client } = fullMock();
    render(<SystemsSection />, { wrapper: connectedWrapper(client) });

    expect(screen.getByTestId("grant-inspector")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("grant-asset-pick-kx/recipes/echo")).toBeInTheDocument(),
    );
    await waitFor(() => expect(screen.getByTestId("grant-table")).toBeInTheDocument());
    const teamGrant = screen.getByTestId("grant-row-kx/teams/demo");
    expect(teamGrant).toBeInTheDocument();
    expect(teamGrant).toHaveTextContent("Root");
  });

  it("selecting an asset resolves each member's warrant on it", async () => {
    const user = userEvent.setup();
    const { client, listTeamMembers } = fullMock();
    render(<SystemsSection />, { wrapper: connectedWrapper(client) });
    await waitFor(() =>
      expect(screen.getByTestId("grant-asset-pick-kx/recipes/echo")).toBeInTheDocument(),
    );
    // Clicking the asset chip lifts the assetRef → the member table resolves warrants.
    await user.click(screen.getByTestId("grant-asset-pick-kx/recipes/echo"));
    await waitFor(() =>
      expect(screen.getByTestId("member-warrant-alice@acme")).toBeInTheDocument(),
    );
    // The member-members call was made WITH an asset_ref at least once.
    await waitFor(() =>
      expect(
        listTeamMembers.mock.calls.some((c) => (c[1] as { assetRef?: string })?.assetRef),
      ).toBe(true),
    );
  });

  it("degrades gracefully when the teams view is not wired", async () => {
    const { client } = makeMockClient({
      listSignatures: async () => [],
      listTeams: async () => {
        throw new KxUnimplemented("ListTeams not wired");
      },
      listRecipes: async () => [],
    });
    render(<SystemsSection />, { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(screen.getByText(/teams not available here/i)).toBeInTheDocument());
    // No crash — the section still renders.
    expect(screen.getByTestId("systems-section")).toBeInTheDocument();
  });

  it("shows an empty state when there are no teams", async () => {
    const { client } = makeMockClient({
      listSignatures: async () => [],
      listTeams: async () => [],
      listRecipes: async () => [],
    });
    render(<SystemsSection />, { wrapper: connectedWrapper(client) });
    await waitFor(() => expect(screen.getByText("No teams")).toBeInTheDocument());
  });

  it("an unknown team surfaces an error notice, not a crash", async () => {
    const { client } = makeMockClient({
      listSignatures: async () => [],
      listTeams: async () => [demoTeam],
      listTeamMembers: async () => {
        throw new KxNotFound("team not found");
      },
      listRecipes: async () => [],
    });
    render(<SystemsSection />, { wrapper: connectedWrapper(client) });
    // The member table renders an ErrorNotice (the honest not-found), not a crash.
    await waitFor(() => expect(screen.getByText("Not found")).toBeInTheDocument());
    expect(screen.getByTestId("systems-section")).toBeInTheDocument();
  });
});
