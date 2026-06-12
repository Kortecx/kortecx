import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
  moteDefHash: "d".repeat(64),
  parents: [],
};

/** The SDK-shaped MoteDetail the mock getMoteDetail resolves. */
const detail = {
  defFound: true,
  moteDefHash: "d".repeat(64),
  stepKind: "model",
  modelId: "qwen3",
  prompt: "summarize the incident",
  promptTruncated: false,
  configSubset: [
    { key: "temperature", value: new TextEncoder().encode("0"), truncated: false, fullLen: 1 },
  ],
  toolContract: { echo: "1" },
  logicRef: "07".repeat(32),
  ndClassName: "PURE",
  effectPatternName: "IdempotentByConstruction",
  criticFor: undefined,
  isTopologyShaper: false,
  schemaVersion: 5,
};

describe("NodeDetailDrawer", () => {
  it("shows the Mote identity + the committed result via GetContent", async () => {
    const payload = new TextEncoder().encode('{"ok":true}');
    const mock = makeMockClient({
      getContent: async () => payload,
      getMoteDetail: async () => detail,
    });
    render(
      <NodeDetailDrawer
        mote={committed}
        motes={[committed]}
        instanceId={INSTANCE}
        onClose={() => {}}
      />,
      { wrapper: connectedWrapper(mock.client) },
    );
    expect(screen.getByTestId("node-detail-drawer")).toBeInTheDocument();
    expect(screen.getByTestId("state-pill")).toHaveTextContent("COMMITTED");
    await waitFor(() =>
      expect(screen.getByTestId("node-detail-result")).toHaveTextContent('"ok": true'),
    );
    expect(mock.getContent).toHaveBeenCalled();
  });

  it("an uncommitted Mote shows 'No committed result yet' (no GetContent call)", () => {
    const mock = makeMockClient();
    const pending: MoteVM = {
      ...committed,
      stateCode: 1,
      resultRef: null,
      committedSeq: null,
      moteDefHash: "",
    };
    render(
      <NodeDetailDrawer
        mote={pending}
        motes={[pending]}
        instanceId={INSTANCE}
        onClose={() => {}}
      />,
      { wrapper: connectedWrapper(mock.client) },
    );
    expect(screen.getByText(/No committed result yet/i)).toBeInTheDocument();
    expect(mock.getContent).not.toHaveBeenCalled();
  });

  it("the Prompt/Params/Tools panes resolve the admitted def (PR-2 inspector)", async () => {
    const mock = makeMockClient({
      getContent: async () => new Uint8Array(),
      getMoteDetail: async () => detail,
    });
    const user = userEvent.setup();
    render(
      <NodeDetailDrawer
        mote={committed}
        motes={[committed]}
        instanceId={INSTANCE}
        onClose={() => {}}
      />,
      { wrapper: connectedWrapper(mock.client) },
    );
    await user.click(screen.getByTestId("inspector-pane-prompt"));
    await waitFor(() =>
      expect(screen.getByTestId("inspector-prompt-text")).toHaveTextContent(
        "summarize the incident",
      ),
    );
    await user.click(screen.getByTestId("inspector-pane-params"));
    expect(screen.getByTestId("inspector-params")).toHaveTextContent("temperature");
    await user.click(screen.getByTestId("inspector-pane-tools"));
    expect(screen.getByTestId("inspector-tool-list")).toHaveTextContent("echo@1");
    expect(screen.getByTestId("inspector-tools")).toHaveTextContent("IdempotentByConstruction");
    // One unary per drawer (content-addressed cache) — pane flips re-use it.
    expect(mock.getMoteDetail).toHaveBeenCalledTimes(1);
  });

  it("a PENDING Mote's def panes stay commit-gated (no RPC)", async () => {
    const mock = makeMockClient();
    const pending: MoteVM = { ...committed, stateCode: 1, moteDefHash: "" };
    const user = userEvent.setup();
    render(
      <NodeDetailDrawer
        mote={pending}
        motes={[pending]}
        instanceId={INSTANCE}
        onClose={() => {}}
      />,
      { wrapper: connectedWrapper(mock.client) },
    );
    await user.click(screen.getByTestId("inspector-pane-prompt"));
    expect(screen.getByText(/Available after commit/i)).toBeInTheDocument();
    expect(mock.getMoteDetail).not.toHaveBeenCalled();
  });

  it("a committed-but-blobless def answers the honest 'not retained' empty", async () => {
    const mock = makeMockClient({
      getMoteDetail: async () => ({ ...detail, defFound: false }),
    });
    const user = userEvent.setup();
    render(
      <NodeDetailDrawer
        mote={committed}
        motes={[committed]}
        instanceId={INSTANCE}
        onClose={() => {}}
      />,
      { wrapper: connectedWrapper(mock.client) },
    );
    await user.click(screen.getByTestId("inspector-pane-prompt"));
    await waitFor(() => expect(screen.getByText(/Definition not retained/i)).toBeInTheDocument());
  });

  it("MoteDag keys the drawer by mote — pane selection resets across nodes", async () => {
    // The drawer is mounted with key={moteId} (MoteDag), so a re-render with a
    // DIFFERENT mote remounts it and the pane returns to Result. Pin the
    // remount semantics by re-rendering with a new key, as MoteDag does.
    const mock = makeMockClient({
      getContent: async () => new TextEncoder().encode("x"),
      getMoteDetail: async () => detail,
    });
    const user = userEvent.setup();
    const wrapper = connectedWrapper(mock.client);
    const other: MoteVM = { ...committed, moteId: "9".repeat(64) };
    const { rerender } = render(
      <NodeDetailDrawer
        key={committed.moteId}
        mote={committed}
        motes={[committed, other]}
        instanceId={INSTANCE}
        onClose={() => {}}
      />,
      { wrapper },
    );
    await user.click(screen.getByTestId("inspector-pane-tools"));
    await waitFor(() => expect(screen.getByTestId("inspector-tools")).toBeInTheDocument());
    rerender(
      <NodeDetailDrawer
        key={other.moteId}
        mote={other}
        motes={[committed, other]}
        instanceId={INSTANCE}
        onClose={() => {}}
      />,
    );
    // The new mote opens on the default Result pane, not the leaked Tools pane.
    expect(screen.queryByTestId("inspector-tools")).not.toBeInTheDocument();
    expect(screen.getByText(/Committed result/i)).toBeInTheDocument();
  });

  it("the Inputs pane resolves parent results via ONE batched read", async () => {
    const parent: MoteVM = {
      ...committed,
      moteId: "e".repeat(64),
      resultRef: "f".repeat(64),
    };
    const child: MoteVM = {
      ...committed,
      moteId: "a".repeat(64),
      parents: [{ parentId: parent.moteId, edgeKind: "data", nonCascade: false }],
    };
    const mock = makeMockClient({
      getMoteDetail: async () => detail,
      getContentBatch: async () => [
        {
          contentRef: "f".repeat(64),
          payload: new TextEncoder().encode("upstream says hi"),
          truncated: false,
          fullSize: 16n,
          missing: false,
        },
      ],
    });
    const user = userEvent.setup();
    render(
      <NodeDetailDrawer
        mote={child}
        motes={[parent, child]}
        instanceId={INSTANCE}
        onClose={() => {}}
      />,
      { wrapper: connectedWrapper(mock.client) },
    );
    await user.click(screen.getByTestId("inspector-pane-inputs"));
    await waitFor(() =>
      expect(screen.getByTestId("inspector-inputs")).toHaveTextContent("upstream says hi"),
    );
    expect(screen.getByTestId("inspector-inputs")).toHaveTextContent("data");
    expect(mock.getContentBatch).toHaveBeenCalledTimes(1);
  });
});
