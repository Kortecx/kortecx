/**
 * RC-SW1 — the Skills panel (the Integrations "Skills" tab) + the App SkillsRail.
 * Pins every designed state (D142): not-wired / empty / populated (wish chips
 * with the ADVISORY registered bit) for the panel, and attached/attachable CHIP
 * controls + the locked-disable on the rail — so a refactor that drops a state
 * or swaps chips for a controlled <select> fails CI here.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const noopMutation = {
  mutate: vi.fn(),
  isPending: false,
  isError: false,
  isSuccess: false,
  data: undefined,
  error: null,
  variables: undefined,
};

const catalog = {
  skills: [
    {
      skillRef: "ab".repeat(16),
      name: "email-triage",
      version: "1",
      description: "triage the inbox",
      instructionsRef: "cd".repeat(32),
      tools: { "gmail/search": "1", "gmail/draft": "1" },
      tags: ["email"],
    },
  ],
  notWired: false,
  isLoading: false,
  isError: false,
  error: null,
  refetch: vi.fn(),
};

let listState: typeof catalog | (Partial<typeof catalog> & { notWired: boolean });

vi.mock("../../src/kx/use-skills", () => ({
  useListSkills: () => listState,
  useSkillForm: () => ({
    form: {
      summary: catalog.skills[0],
      wishes: [
        { toolId: "gmail/search", toolVersion: "1", registered: true },
        { toolId: "gmail/draft", toolVersion: "1", registered: false },
      ],
      instructionsPreview: "# Triage",
      previewTruncated: false,
    },
    isLoading: false,
    isError: false,
    error: null,
  }),
  useAddSkill: () => noopMutation,
  useRemoveSkill: () => noopMutation,
}));

vi.mock("../../src/kx/use-apps", () => ({
  useSaveApp: () => noopMutation,
}));

import { SkillsRail } from "../../src/components/apps/SkillsRail";
import { SkillsPanel } from "../../src/components/tools/SkillsPanel";

describe("Skills — the catalog panel (RC-SW1)", () => {
  it("renders the populated list with CHIP controls + the add form", () => {
    listState = catalog;
    render(<SkillsPanel />);
    expect(screen.getByTestId("skills-panel")).toBeInTheDocument();
    expect(screen.getByTestId("skill-email-triage")).toHaveTextContent("email-triage@1");
    expect(screen.getByTestId("skill-email-triage")).toHaveTextContent("2 tool wish(es)");
    // CHIP controls, never a controlled <select> (the UI-3 e2e gotcha).
    expect(screen.getByTestId("skill-show-email-triage").tagName).toBe("BUTTON");
    expect(screen.getByTestId("skill-remove-email-triage").tagName).toBe("BUTTON");
    expect(screen.getByTestId("skill-add-form")).toBeInTheDocument();
    expect(screen.getByTestId("skill-add-submit")).toBeInTheDocument();
  });

  it("renders the honest empty + not-wired states", () => {
    listState = { ...catalog, skills: [], notWired: false };
    const { unmount } = render(<SkillsPanel />);
    expect(screen.getByText("No skills in the catalog")).toBeInTheDocument();
    unmount();

    listState = { ...catalog, skills: [], notWired: true, isError: true };
    render(<SkillsPanel />);
    expect(screen.getByText("Skill catalog not available")).toBeInTheDocument();
    // No add form on a gateway without the catalog (don't fake gaps).
    expect(screen.queryByTestId("skill-add-form")).toBeNull();
  });

  it("surfaces a malformed-manifest JSON parse error instead of silently no-op'ing", async () => {
    const { fireEvent } = await import("@testing-library/react");
    listState = catalog;
    render(<SkillsPanel />);
    fireEvent.change(screen.getByTestId("skill-add-manifest"), {
      target: { value: "{ not valid json" },
    });
    fireEvent.click(screen.getByTestId("skill-add-submit"));
    // The local parse error shows (D142: every state designed) — the button is not dead.
    expect(screen.getByTestId("skill-add-error")).toHaveTextContent(/not valid JSON/i);
  });
});

describe("Apps — the SkillsRail attach control (RC-SW1)", () => {
  const envelope = {
    schema: "kortecx.app/v1",
    name: "triager",
    blueprint: { steps: [] },
    references: {
      skills: [{ name: "research-summarize", instructions_ref: "ee".repeat(32) }],
    },
  };

  it("shows attached + attachable skills as CHIPs", () => {
    listState = catalog;
    render(<SkillsRail handle="apps/local/triager" envelope={envelope} locked={false} />);
    expect(screen.getByTestId("app-skills-rail")).toBeInTheDocument();
    expect(screen.getByTestId("app-skill-detach-research-summarize").tagName).toBe("BUTTON");
    expect(screen.getByTestId("app-skill-attach-email-triage").tagName).toBe("BUTTON");
    expect(screen.getByTestId("app-skill-attach-email-triage")).not.toBeDisabled();
  });

  it("disables the controls with the reason when the App is locked", () => {
    listState = catalog;
    render(<SkillsRail handle="apps/local/triager" envelope={envelope} locked={true} />);
    expect(screen.getByTestId("app-skills-rail")).toHaveTextContent("App is locked");
    expect(screen.getByTestId("app-skill-detach-research-summarize")).toBeDisabled();
    expect(screen.getByTestId("app-skill-attach-email-triage")).toBeDisabled();
  });
});
