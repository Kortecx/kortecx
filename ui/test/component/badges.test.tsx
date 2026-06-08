import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AnomalyBadge } from "../../src/components/AnomalyBadge";
import { NdClassBadge } from "../../src/components/NdClassBadge";

describe("NdClassBadge", () => {
  it.each([
    [1, "pure", "PURE"],
    [2, "read-only-nondet", "READ_ONLY_NONDET"],
    [3, "world-mutating", "WORLD_MUTATING"],
    [0, "unknown", "UNKNOWN"],
    [77, "unknown", "UNKNOWN"],
  ])("nd_class %i → tone %s", (code, tone, label) => {
    render(<NdClassBadge ndClass={code} />);
    const badge = screen.getByTestId("nd-badge");
    expect(badge).toHaveAttribute("data-tone", tone);
    expect(badge).toHaveTextContent(label);
  });
});

describe("AnomalyBadge", () => {
  it("renders nothing for a healthy Mote", () => {
    const { container } = render(<AnomalyBadge anomaly={null} />);
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByTestId("anomaly-badge")).not.toBeInTheDocument();
  });
  it("renders nothing for UNSPECIFIED (0)", () => {
    render(<AnomalyBadge anomaly={0} />);
    expect(screen.queryByTestId("anomaly-badge")).not.toBeInTheDocument();
  });
  it.each([
    [1, "EFFECT_STAGED_THEN_REPUDIATED"],
    [2, "QUARANTINED_AT_LEAST_ONCE_EFFECT"],
    [99, "UNKNOWN_ANOMALY"],
  ])("anomaly %i shows a warning badge", (code, label) => {
    render(<AnomalyBadge anomaly={code} />);
    expect(screen.getByTestId("anomaly-badge")).toHaveTextContent(label);
  });
});
