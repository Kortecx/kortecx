import { ErrorCode } from "@kortecx/sdk/web";
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useChat } from "../../src/kx/use-chat";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";
import { mote, projection } from "../mocks/projection-fixtures";

const INSTANCE = "ab".repeat(16);
const TERMINAL = "ee".repeat(32);
const REF = "cd".repeat(32);
const OPTS = { handle: "kx/recipes/chat", promptKey: "prompt" };

describe("useChat — the Invoke→poll→GetContent round-trip", () => {
  it("completes a turn: assistant renders the decoded terminal result", async () => {
    const { client, invoke, getContent } = makeMockClient({
      invoke: async () => ({ instanceId: INSTANCE, terminalMoteId: TERMINAL }),
      getProjection: async () =>
        projection([mote({ moteId: TERMINAL, stateCode: 3, resultRef: REF, committedSeq: 2 })], {
          currentSeq: 2,
        }),
      getContent: async () => new TextEncoder().encode("the answer"),
    });
    const { result } = renderHook(() => useChat(OPTS), { wrapper: connectedWrapper(client) });

    await act(async () => {
      await result.current.send("hello");
    });

    await waitFor(() => expect(result.current.thread.messages[1]?.status).toBe("done"));
    expect(result.current.thread.messages[0]).toMatchObject({ role: "user", text: "hello" });
    expect(result.current.thread.messages[1]).toMatchObject({
      role: "assistant",
      text: "the answer",
    });
    expect(invoke).toHaveBeenCalledWith("kx/recipes/chat", { prompt: "hello" });
    expect(getContent).toHaveBeenCalledWith(REF, INSTANCE);
    expect(result.current.busy).toBe(false);
  });

  it("a FAILED terminal Mote fails the turn (no content fetched)", async () => {
    const { client, getContent } = makeMockClient({
      invoke: async () => ({ instanceId: INSTANCE, terminalMoteId: TERMINAL }),
      getProjection: async () =>
        projection([mote({ moteId: TERMINAL, stateCode: 4 })], { currentSeq: 3 }),
    });
    const { result } = renderHook(() => useChat(OPTS), { wrapper: connectedWrapper(client) });

    await act(async () => {
      await result.current.send("will fail");
    });

    await waitFor(() => expect(result.current.thread.messages[1]?.status).toBe("failed"));
    expect(result.current.thread.messages[1]?.error?.title).toMatch(/run failed/i);
    expect(getContent).not.toHaveBeenCalled();
  });

  it("degrades when the chat recipe/model is not provisioned (Invoke UNIMPLEMENTED)", async () => {
    const { client } = makeMockClient({
      invoke: async () => {
        throw Object.assign(new Error("not wired"), { code: ErrorCode.Unimplemented });
      },
    });
    const { result } = renderHook(() => useChat(OPTS), { wrapper: connectedWrapper(client) });

    await act(async () => {
      await result.current.send("hi");
    });

    await waitFor(() => expect(result.current.degraded).not.toBeNull());
    expect(result.current.degraded?.kind).toBe("not-wired");
    expect(result.current.thread.messages[1]?.status).toBe("failed");
  });

  it("reset clears the thread and degrade state", async () => {
    const { client } = makeMockClient({
      invoke: async () => ({ instanceId: INSTANCE, terminalMoteId: TERMINAL }),
      getProjection: async () =>
        projection([mote({ moteId: TERMINAL, stateCode: 3, resultRef: REF })]),
      getContent: async () => new TextEncoder().encode("x"),
    });
    const { result } = renderHook(() => useChat(OPTS), { wrapper: connectedWrapper(client) });
    await act(async () => {
      await result.current.send("hi");
    });
    await waitFor(() => expect(result.current.thread.messages.length).toBeGreaterThan(0));
    act(() => result.current.reset());
    expect(result.current.thread.messages).toHaveLength(0);
  });
});
