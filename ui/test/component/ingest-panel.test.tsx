/**
 * PR-6: the Ingest panel's multimodal FILE path — staging picked files + the
 * submit-enable logic. The full byte-read → `IngestDocuments` round-trip is covered by
 * the datasets e2e (real browser File.arrayBuffer); here we assert the deterministic UI
 * wiring (staging, removal, and that a file alone — with a name — enables submit).
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { IngestPanel } from "../../src/components/datasets/IngestPanel";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

function renderPanel() {
  const { client } = makeMockClient();
  return render(<IngestPanel />, { wrapper: connectedWrapper(client) });
}

describe("IngestPanel multimodal file ingest (PR-6)", () => {
  it("stages a picked file, enables submit with a name, and removes it", () => {
    renderPanel();
    // A name alone is not enough — nothing to ingest yet.
    fireEvent.change(screen.getByTestId("dataset-ingest-name"), { target: { value: "corpus" } });
    expect(screen.getByTestId("dataset-ingest-submit")).toBeDisabled();

    // Pick a file → it stages, and submit enables (a file IS content to ingest).
    const file = new File([new Uint8Array([1, 2, 3, 4])], "pixel.png", { type: "image/png" });
    fireEvent.change(screen.getByTestId("dataset-ingest-file-input"), {
      target: { files: [file] },
    });
    expect(screen.getByTestId("dataset-ingest-file-pixel.png")).toBeInTheDocument();
    expect(screen.getByTestId("dataset-ingest-submit")).toBeEnabled();

    // Remove it → back to nothing-to-ingest → submit disabled again.
    fireEvent.click(screen.getByTestId("dataset-ingest-file-remove-pixel.png"));
    expect(screen.queryByTestId("dataset-ingest-file-pixel.png")).toBeNull();
    expect(screen.getByTestId("dataset-ingest-submit")).toBeDisabled();
  });
});
