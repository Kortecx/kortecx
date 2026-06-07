import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatePill } from "../../src/components/StatePill";

describe("StatePill", () => {
  it.each([
    [1, "pending", "PENDING"],
    [2, "scheduled", "SCHEDULED"],
    [3, "committed", "COMMITTED"],
    [4, "failed", "FAILED"],
    [5, "repudiated", "REPUDIATED"],
    [6, "inconsistent", "INCONSISTENT"],
    [0, "unknown", "UNKNOWN"],
    [99, "unknown", "UNKNOWN"],
  ])("state %i renders tone %s", (code, tone, label) => {
    render(<StatePill stateCode={code} />);
    const pill = screen.getByTestId("state-pill");
    expect(pill).toHaveAttribute("data-tone", tone);
    expect(pill).toHaveTextContent(label);
  });
});
