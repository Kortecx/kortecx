/**
 * PR-A: `MessageSources` renders a settled grounded turn's citations (label +
 * content-address chip + snippet) as a compact disclosure, and renders NOTHING when
 * the turn is unsettled or ungrounded — a plain answer never grows a faked citation.
 */

import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MessageSources } from "../../src/components/chat/MessageSources";
import { CONTEXT_ITEMS_KEY } from "../../src/lib/context-items";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

function encodeItem(name: string, refByte: number): Uint8Array {
  const nameBytes = new TextEncoder().encode(name);
  const buf = new Uint8Array(4 + nameBytes.length + 32);
  new DataView(buf.buffer).setUint32(0, nameBytes.length, true);
  buf.set(nameBytes, 4);
  buf.fill(refByte, 4 + nameBytes.length);
  return buf;
}
const hex = (byte: number) => byte.toString(16).padStart(2, "0").repeat(32);

function groundedClient() {
  return makeMockClient({
    getMoteDetail: async () => ({
      configSubset: [
        {
          key: CONTEXT_ITEMS_KEY,
          value: encodeItem("launch.md", 0x11),
          truncated: false,
          fullLen: 45,
        },
      ],
    }),
    getContentBatch: async () => [
      {
        contentRef: hex(0x11),
        missing: false,
        truncated: false,
        fullSize: 30,
        payload: new TextEncoder().encode("The codename is FALCON-NINE-ZULU."),
      },
    ],
  }).client;
}

describe("MessageSources (PR-A)", () => {
  it("renders the grounded sources with their snippet + digest chip", async () => {
    render(<MessageSources instanceId="run1" moteId="mote1" active />, {
      wrapper: connectedWrapper(groundedClient()),
    });
    // The snippet resolves via the SECOND query (GetContentBatch), after the sources
    // disclosure first appears — wait for the grounded text itself.
    await waitFor(() =>
      expect(screen.getByTestId("chat-source-detail")).toHaveTextContent("FALCON-NINE-ZULU"),
    );
    expect(screen.getByTestId("chat-sources")).toBeInTheDocument();
    expect(screen.getByTestId("chat-source")).toBeInTheDocument();
    expect(screen.getByText("launch.md")).toBeInTheDocument();
    expect(screen.getByTestId("digest-chip")).toBeInTheDocument();
  });

  it("renders nothing while the turn is unsettled (active=false)", async () => {
    const { container } = render(
      <MessageSources instanceId="run1" moteId="mote1" active={false} />,
      { wrapper: connectedWrapper(groundedClient()) },
    );
    // No RPC, no disclosure — an in-flight answer shows no citations.
    expect(screen.queryByTestId("chat-sources")).toBeNull();
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing for an ungrounded answer (no folded refs)", async () => {
    const client = makeMockClient({
      getMoteDetail: async () => ({ configSubset: [] }),
    }).client;
    render(<MessageSources instanceId="run1" moteId="mote1" active />, {
      wrapper: connectedWrapper(client),
    });
    // Let the (empty) resolve settle; still no disclosure.
    await new Promise((r) => setTimeout(r, 15));
    expect(screen.queryByTestId("chat-sources")).toBeNull();
  });
});
