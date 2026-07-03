/**
 * ReactProgress — the PR-9c-1 "Actions taken" summary. The agent-runner surfaces
 * the AUDITED action set (the chain's settled `tool` turns) as an honest summary
 * derived from the same durable `ListReactTurns` facts — no new RPC/state.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ReactProgress } from "../../src/components/chat/ReactProgress";
import type { ReactTurnVM } from "../../src/kx/use-react-progress";

function turn(
  turn: number,
  branch: string,
  toolId = "",
  toolVersion = "",
  callIndex = 0,
): ReactTurnVM {
  return {
    turn,
    branch,
    toolId,
    toolVersion,
    turnMoteId: "ab",
    maxTurns: 8,
    rejectionReason: "",
    callIndex,
    grantedTools: [],
    secretScopeNames: [],
  };
}

describe("ReactProgress — actions taken summary", () => {
  it("summarizes the fired tool turns as an audited action set", () => {
    render(
      <ReactProgress
        turns={[
          turn(0, "tool", "mcp-echo/echo", "1"),
          turn(1, "pending"),
          turn(2, "tool", "fs-list", "1"),
          turn(3, "answer"),
        ]}
      />,
    );
    const summary = screen.getByTestId("react-actions");
    expect(summary.textContent).toContain("Actions taken: 2");
    expect(summary.textContent).toContain("mcp-echo/echo@1");
    expect(summary.textContent).toContain("fs-list@1");
  });

  it("dedupes repeated tools but counts every action", () => {
    render(
      <ReactProgress
        turns={[turn(0, "tool", "mcp-echo/echo", "1"), turn(1, "tool", "mcp-echo/echo", "1")]}
      />,
    );
    const summary = screen.getByTestId("react-actions");
    expect(summary.textContent).toContain("Actions taken: 2");
    // one distinct tool listed
    expect(summary.textContent).toContain("(mcp-echo/echo@1)");
  });

  it("renders no actions summary when the agent only reasons + answers", () => {
    render(<ReactProgress turns={[turn(0, "pending"), turn(1, "answer")]} />);
    expect(screen.queryByTestId("react-actions")).toBeNull();
  });

  it("surfaces N parallel tools fired in ONE turn (T-MULTI-ELEMENT-TOOLCALLS)", () => {
    // A multi-tool turn fans into N "tool" rows sharing the turn, call-indexed —
    // the trajectory must show ALL of them, not collapse to one.
    render(
      <ReactProgress
        turns={[
          turn(0, "tool", "mcp-echo/echo", "1", 0),
          turn(0, "tool", "fs-list", "1", 1),
          turn(1, "answer"),
        ]}
      />,
    );
    const summary = screen.getByTestId("react-actions");
    expect(summary.textContent).toContain("Actions taken: 2");
    expect(summary.textContent).toContain("mcp-echo/echo@1");
    expect(summary.textContent).toContain("fs-list@1");
  });
});
