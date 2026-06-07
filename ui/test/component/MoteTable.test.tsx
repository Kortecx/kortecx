import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MoteTable } from "../../src/components/MoteTable";
import { toProjectionVM } from "../../src/kx/use-projection";
import {
  allStatesProjection,
  largeProjection,
  mote,
  projection,
} from "../mocks/projection-fixtures";

describe("MoteTable", () => {
  it("renders one row per Mote across all states", () => {
    render(<MoteTable projection={toProjectionVM(allStatesProjection())} />);
    expect(screen.getAllByTestId("mote-row")).toHaveLength(7);
  });

  it("empty projection shows an empty state, not a table", () => {
    render(<MoteTable projection={toProjectionVM(projection([]))} />);
    expect(screen.getByTestId("empty-state")).toBeInTheDocument();
    expect(screen.queryByTestId("mote-table")).not.toBeInTheDocument();
  });

  it("a single Mote renders one row", () => {
    render(<MoteTable projection={toProjectionVM(projection([mote({ stateCode: 3 })]))} />);
    expect(screen.getAllByTestId("mote-row")).toHaveLength(1);
  });

  it("maps each state code to its pill tone (incl. UNKNOWN for out-of-range)", () => {
    const vm = toProjectionVM(projection([mote({ stateCode: 3 }), mote({ stateCode: 99 })]));
    render(<MoteTable projection={vm} />);
    const pills = screen.getAllByTestId("state-pill");
    expect(pills[0]).toHaveAttribute("data-tone", "committed");
    expect(pills[1]).toHaveAttribute("data-tone", "unknown");
  });

  it("surfaces an anomaly badge only for an anomalous Mote", () => {
    const vm = toProjectionVM(
      projection([mote({ stateCode: 6, anomaly: 2 }), mote({ stateCode: 3, anomaly: null })]),
    );
    render(<MoteTable projection={vm} />);
    expect(screen.getAllByTestId("anomaly-badge")).toHaveLength(1);
  });

  it("renders 5000 Motes within the perf budget", () => {
    const vm = toProjectionVM(largeProjection(5000));
    const t0 = performance.now();
    render(<MoteTable projection={vm} />);
    const elapsed = performance.now() - t0;
    expect(screen.getAllByTestId("mote-row")).toHaveLength(5000);
    // Generous jsdom budget — guards against accidental O(n^2) work in the table.
    expect(elapsed).toBeLessThan(4000);
  });
});
