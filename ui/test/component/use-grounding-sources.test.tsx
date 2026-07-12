/**
 * PR-A: `useGroundingSources` resolves a settled chat-rag turn's grounded refs
 * (folded into the answer Mote's `config_subset["kx.context.items"]`) into cited
 * snippets over the SHIPPED `GetMoteDetail` → `GetContentBatch` — no new RPC.
 * Commit-gated, and honest-degrades to no sources on an ungrounded turn or an old
 * gateway.
 */

import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useGroundingSources } from "../../src/kx/use-grounding-sources";
import { CONTEXT_ITEMS_KEY } from "../../src/lib/context-items";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

/** Encode one grounded item exactly as the runtime does (32-byte ref = `refByte`). */
function encodeItem(name: string, refByte: number): Uint8Array {
  const nameBytes = new TextEncoder().encode(name);
  const buf = new Uint8Array(4 + nameBytes.length + 32);
  new DataView(buf.buffer).setUint32(0, nameBytes.length, true);
  buf.set(nameBytes, 4);
  buf.fill(refByte, 4 + nameBytes.length);
  return buf;
}
const hex = (byte: number) => byte.toString(16).padStart(2, "0").repeat(32);

describe("useGroundingSources (PR-A)", () => {
  it("resolves the answer Mote's folded refs into cited snippets (GetMoteDetail → GetContentBatch)", async () => {
    const ref = hex(0x11);
    const { client, getMoteDetail, getContentBatch } = makeMockClient({
      getMoteDetail: async () => ({
        configSubset: [
          {
            key: CONTEXT_ITEMS_KEY,
            value: encodeItem("spec.md", 0x11),
            truncated: false,
            fullLen: 43,
          },
        ],
      }),
      getContentBatch: async () => [
        {
          contentRef: ref,
          missing: false,
          truncated: false,
          fullSize: 11,
          payload: new TextEncoder().encode("hello world"),
        },
      ],
    });
    const { result } = renderHook(() => useGroundingSources("run1", "mote1", true), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.sources).toHaveLength(1));
    expect(result.current.sources[0]).toMatchObject({
      ref,
      label: "spec.md",
      snippet: "hello world",
      missing: false,
    });
    // The exact terminal Mote is read (no new RPC); the batch resolves the refs.
    expect(getMoteDetail).toHaveBeenCalledWith("run1", "mote1");
    expect(getContentBatch).toHaveBeenCalled();
  });

  it("flags truncation honestly when the folded ref list overran the detail cap", async () => {
    const ref = hex(0x11);
    const { client } = makeMockClient({
      getMoteDetail: async () => ({
        configSubset: [
          {
            key: CONTEXT_ITEMS_KEY,
            value: encodeItem("spec.md", 0x11),
            truncated: true,
            fullLen: 9000,
          },
        ],
      }),
      getContentBatch: async () => [
        {
          contentRef: ref,
          missing: false,
          truncated: false,
          fullSize: 3,
          payload: new TextEncoder().encode("hi"),
        },
      ],
    });
    const { result } = renderHook(() => useGroundingSources("run1", "mote1", true), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(result.current.sources).toHaveLength(1));
    expect(result.current.truncated).toBe(true);
  });

  it("returns no sources for an ungrounded answer (no context-items key)", async () => {
    const { client, getMoteDetail } = makeMockClient({
      getMoteDetail: async () => ({
        configSubset: [
          { key: "kx.other", value: new Uint8Array([1, 2]), truncated: false, fullLen: 2 },
        ],
      }),
    });
    const { result } = renderHook(() => useGroundingSources("run1", "mote1", true), {
      wrapper: connectedWrapper(client),
    });
    await waitFor(() => expect(getMoteDetail).toHaveBeenCalled());
    expect(result.current.sources).toEqual([]);
  });

  it("is disabled until the turn settles (enabled=false ⇒ no RPC fires)", async () => {
    const { client, getMoteDetail } = makeMockClient({
      getMoteDetail: async () => ({ configSubset: [] }),
    });
    const { result } = renderHook(() => useGroundingSources("run1", "mote1", false), {
      wrapper: connectedWrapper(client),
    });
    await new Promise((r) => setTimeout(r, 15));
    expect(getMoteDetail).not.toHaveBeenCalled();
    expect(result.current.sources).toEqual([]);
  });

  it("honest-degrades to no sources when GetMoteDetail is unimplemented (old gateway)", async () => {
    const { client } = makeMockClient({
      getMoteDetail: async () => {
        throw new Error("UNIMPLEMENTED");
      },
    });
    const { result } = renderHook(() => useGroundingSources("run1", "mote1", true), {
      wrapper: connectedWrapper(client),
    });
    await new Promise((r) => setTimeout(r, 15));
    expect(result.current.sources).toEqual([]);
  });
});
