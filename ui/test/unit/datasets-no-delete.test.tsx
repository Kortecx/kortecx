/**
 * PR-C2 GR15 guard — the Datasets re-skin adopts the reference app's card language,
 * but the reference `DatasetCard` ships a red "Delete" button and OUR gateway has NO
 * `DeleteDataset`/`DropDataset` RPC. A faked, non-functional delete would violate
 * GR15 (don't-fake-gaps), so this pins the ABSENCE of any delete affordance: a future
 * copy-paste re-skin that smuggles one back in fails CI here (and in datasets.spec.ts).
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// Mock the datasets hook so the panel renders seeded corpora without a live client.
vi.mock("../../src/kx/use-datasets", () => ({
  useDatasets: () => ({
    data: [
      { datasetId: "demo-corpus", name: "demo-corpus", docCount: 3, dim: 384 },
      { datasetId: "notes", name: "notes", docCount: 1, dim: 384 },
    ],
    isLoading: false,
    isError: false,
    error: null,
  }),
}));

import { DatasetsPanel } from "../../src/components/datasets/DatasetsPanel";

describe("Datasets — no faked delete (GR15 guard)", () => {
  it("renders the corpora but exposes NO delete/drop affordance", () => {
    const { container } = render(
      <DatasetsPanel selectedDataset="demo-corpus" onSelect={() => {}} />,
    );

    // Sanity — the seeded corpus actually rendered, so the absence below is meaningful.
    expect(screen.getByTestId("dataset-pick-demo-corpus")).toBeInTheDocument();

    // No delete control of any kind: no DeleteDataset RPC exists on the gateway.
    expect(screen.queryByRole("button", { name: /delete|remove|drop/i })).toBeNull();
    expect(screen.queryByTestId("dataset-delete")).toBeNull();
    // A future delete button would conventionally use a `dataset-delete*` testid.
    expect(container.querySelectorAll('[data-testid^="dataset-delete"]')).toHaveLength(0);
  });
});
