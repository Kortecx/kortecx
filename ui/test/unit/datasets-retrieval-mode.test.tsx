/**
 * RC4a — the QueryPanel exposes a Hybrid/Dense retrieval-mode chip (button controls,
 * never a controlled `<select>` — the Playwright `selectOption` gotcha) and renders a
 * chunked hit's passage position within its parent document.
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// Mock the query hooks so the panel renders without a live client.
vi.mock("../../src/kx/use-datasets", () => ({
  useDatasetQuery: () => ({
    data: [
      {
        contentRef: "ab".repeat(32),
        content: new Uint8Array(),
        score: 0.9,
        text: "the relevant passage",
        parentRef: "cd".repeat(32),
        chunkIndex: 1,
        chunkCount: 3,
      },
    ],
    isFetching: false,
    isError: false,
    error: null,
  }),
}));
vi.mock("../../src/kx/use-fuzzy-discovery", () => ({
  useFuzzyDiscovery: () => ({ data: [], isFetching: false, isError: false, error: null }),
}));

import { QueryPanel } from "../../src/components/datasets/QueryPanel";

describe("QueryPanel — RC4a retrieval mode + chunk provenance", () => {
  it("offers Hybrid/Dense chips (hybrid default) and shows a hit's chunk position", () => {
    render(<QueryPanel dataset="corpus" />);

    const hybrid = screen.getByTestId("dataset-retrieval-hybrid");
    const dense = screen.getByTestId("dataset-retrieval-dense");
    // Hybrid (BM25 + dense) is the recommended default.
    expect(hybrid).toHaveAttribute("aria-pressed", "true");
    expect(dense).toHaveAttribute("aria-pressed", "false");

    // The control is button chips, not a controlled <select> (Playwright gotcha).
    fireEvent.click(dense);
    expect(dense).toHaveAttribute("aria-pressed", "true");
    expect(hybrid).toHaveAttribute("aria-pressed", "false");

    // A chunked hit (chunkCount > 1) shows its 1-based passage position.
    expect(screen.getByTestId("dataset-hit-chunk").textContent).toContain("chunk 2/3");
  });
});
