import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { NodeDetailDrawer } from "../../src/components/dag/NodeDetailDrawer";
import type { MoteVM } from "../../src/kx/use-projection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const INSTANCE = "c".repeat(32);

const committed: MoteVM = {
  moteId: "a".repeat(64),
  stateCode: 3, // COMMITTED
  ndClass: 1, // PURE
  promotion: 1,
  resultRef: "b".repeat(64),
  committedSeq: 12,
  anomaly: null,
  parents: [],
};

describe("NodeDetailDrawer", () => {
  it("shows the Mote identity + the committed result via GetContent", async () => {
    const payload = new TextEncoder().encode('{"ok":true}');
    const mock = makeMockClient({ getContent: async () => payload });
    render(<NodeDetailDrawer mote={committed} instanceId={INSTANCE} onClose={() => {}} />, {
      wrapper: connectedWrapper(mock.client),
    });
    expect(screen.getByTestId("node-detail-drawer")).toBeInTheDocument();
    expect(screen.getByTestId("state-pill")).toHaveTextContent("COMMITTED");
    await waitFor(() =>
      expect(screen.getByTestId("node-detail-result")).toHaveTextContent('"ok": true'),
    );
    expect(mock.getContent).toHaveBeenCalled();
  });

  it("an uncommitted Mote shows 'No committed result yet' (no GetContent call)", () => {
    const mock = makeMockClient();
    const pending: MoteVM = { ...committed, stateCode: 1, resultRef: null, committedSeq: null };
    render(<NodeDetailDrawer mote={pending} instanceId={INSTANCE} onClose={() => {}} />, {
      wrapper: connectedWrapper(mock.client),
    });
    expect(screen.getByText(/No committed result yet/i)).toBeInTheDocument();
    expect(mock.getContent).not.toHaveBeenCalled();
  });
});
