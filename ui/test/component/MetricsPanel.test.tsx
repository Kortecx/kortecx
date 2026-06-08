import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MetricsPanel } from "../../src/components/metrics/MetricsPanel";
import { toProjectionVM } from "../../src/kx/use-projection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";
import { mote, projection } from "../mocks/projection-fixtures";

describe("MetricsPanel", () => {
  it("renders derived metric cards + the state breakdown for a projection", async () => {
    const { client } = makeMockClient({ listSignatures: async () => [] });
    const vm = toProjectionVM(
      projection(
        [
          mote({ stateCode: 3, committedSeq: 1 }),
          mote({ stateCode: 3, committedSeq: 4 }),
          mote({ stateCode: 4 }),
        ],
        { currentSeq: 6 },
      ),
    );
    render(<MetricsPanel projection={vm} />, { wrapper: connectedWrapper(client) });

    expect(screen.getByText("Motes")).toBeInTheDocument();
    expect(screen.getByText("Committed")).toBeInTheDocument();
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Success rate")).toBeInTheDocument();
    expect(screen.getByTestId("state-breakdown")).toBeInTheDocument();
    // Gateway health pill resolves to live (listSignatures returns []).
    await waitFor(() =>
      expect(screen.getByTestId("health-indicator")).toHaveAttribute("data-health", "live"),
    );
  });

  it("without a projection shows only the health card + a hint", () => {
    const { client } = makeMockClient({ listSignatures: async () => [] });
    render(<MetricsPanel />, { wrapper: connectedWrapper(client) });
    expect(screen.getByText("Gateway")).toBeInTheDocument();
    expect(screen.getByText(/select a run to see its metrics/i)).toBeInTheDocument();
    expect(screen.queryByTestId("state-breakdown")).not.toBeInTheDocument();
  });
});
