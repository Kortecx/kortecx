import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ReactProgress } from "../../src/components/chat/ReactProgress";
import type { ReactTurnVM } from "../../src/kx/use-react-progress";

function vm(over: Partial<ReactTurnVM>): ReactTurnVM {
  return {
    turn: 0,
    branch: "pending",
    toolId: "",
    toolVersion: "",
    turnMoteId: "aa".repeat(32),
    maxTurns: 8,
    rejectionReason: "",
    callIndex: 0,
    grantedTools: [],
    secretScopeNames: [],
    ...over,
  };
}

describe("ReactProgress (PR-3/A2 rejected disclosure)", () => {
  it("shows the empty starting state", () => {
    render(<ReactProgress turns={[]} />);
    expect(screen.getByTestId("react-progress")).toHaveTextContent(/agent loop starting/i);
  });

  it("renders a rejected turn as a disclosure that reveals the fail-closed reason", () => {
    const turns = [
      vm({
        turn: 0,
        branch: "rejected",
        rejectionReason: "the arguments for `mcp-echo/echo@1` do not match its inputSchema",
      }),
      vm({ turn: 1, branch: "answer" }),
    ];
    render(<ReactProgress turns={turns} />);
    // The chip summary names the rejection; the reason panel carries the detail.
    const chip = screen.getByTestId("react-turn-0");
    expect(chip).toHaveAttribute("data-branch", "rejected");
    expect(chip).toHaveTextContent(/rejected/i);
    const reason = screen.getByTestId("react-turn-0-reason");
    expect(reason).toHaveTextContent(/do not match its inputSchema/);
    // The recovery is visible: a following answer turn renders normally.
    expect(screen.getByTestId("react-turn-1")).toHaveTextContent(/answer/i);
    // The disclosure is keyboard-toggleable (native <details>).
    fireEvent.click(chip.querySelector("summary") as Element);
  });

  it("a rejected turn without a reason degrades to a plain chip (old server)", () => {
    render(<ReactProgress turns={[vm({ turn: 0, branch: "rejected", rejectionReason: "" })]} />);
    const chip = screen.getByTestId("react-turn-0");
    expect(chip).toHaveTextContent(/rejected/i);
    expect(screen.queryByTestId("react-turn-0-reason")).toBeNull();
  });

  it("W2: a dead-letter AFTER firing tools shows the looped-on-tools hint", () => {
    // The hook keeps one VM per turn (newest fact wins), so the last tool turn —
    // where the DeadLettered branch is appended at the same index — surfaces as
    // `dead_lettered`; the earlier turns remain `tool`.
    const turns = [
      vm({ turn: 0, branch: "tool", toolId: "mcp-echo/echo", toolVersion: "1" }),
      vm({ turn: 1, branch: "tool", toolId: "mcp-echo/echo", toolVersion: "1" }),
      vm({ turn: 2, branch: "dead_lettered" }),
    ];
    render(<ReactProgress turns={turns} />);
    expect(screen.getByTestId("react-deadletter-hint")).toHaveTextContent(
      /exhausted its tool-call budget without settling/i,
    );
  });

  it("an all-rejected dead-letter (no tools fired) shows NO looped-on-tools hint", () => {
    const turns = [
      vm({ turn: 0, branch: "rejected", rejectionReason: "not granted" }),
      vm({ turn: 1, branch: "dead_lettered" }),
    ];
    render(<ReactProgress turns={turns} />);
    expect(screen.queryByTestId("react-deadletter-hint")).toBeNull();
  });

  it("surfaces the chain's warrant grants (governance observability, names/refs only)", () => {
    const turns = [
      vm({
        turn: 0,
        branch: "tool",
        toolId: "gmail/search",
        toolVersion: "1",
        grantedTools: ["gmail/search@1"],
        secretScopeNames: ["KX_GMAIL_CREDENTIAL"],
      }),
      vm({
        turn: 1,
        branch: "answer",
        grantedTools: ["gmail/search@1"],
        secretScopeNames: ["KX_GMAIL_CREDENTIAL"],
      }),
    ];
    render(<ReactProgress turns={turns} />);
    const grants = screen.getByTestId("react-grants");
    expect(grants).toHaveTextContent(/tools \[gmail\/search@1\]/);
    expect(grants).toHaveTextContent(/secrets \[KX_GMAIL_CREDENTIAL\]/);
  });

  it("shows no governance line when the chain carries no grants (old server / empty)", () => {
    render(<ReactProgress turns={[vm({ turn: 0, branch: "answer" })]} />);
    expect(screen.queryByTestId("react-grants")).toBeNull();
  });
});
