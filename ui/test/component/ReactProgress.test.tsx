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
});
