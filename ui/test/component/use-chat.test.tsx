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

  it("a tool-attached turn submits a MODEL-step workflow and resolves via the react chain salt", async () => {
    const SALT = "77".repeat(32);
    let submitted: unknown;
    const { client, submitWorkflow, invoke, getContent } = makeMockClient({
      submitWorkflow: async (req: unknown) => {
        submitted = req;
        return { instanceId: INSTANCE, reactChainSalt: SALT, recipeFingerprint: "ff".repeat(32) };
      },
      // The salt-scoped ListReactTurns poll settles on the answer turn.
      listReactTurns: async () => ({
        turns: [
          {
            turn: 0,
            branch: "answer",
            toolId: "",
            toolVersion: "",
            turnMoteId: TERMINAL,
            maxTurns: 8,
            rejectionReason: "",
            callIndex: 0,
            grantedTools: [],
            secretScopeNames: [],
          },
        ],
        hasMore: false,
      }),
      getProjection: async () =>
        projection([mote({ moteId: TERMINAL, stateCode: 3, resultRef: REF, committedSeq: 2 })], {
          currentSeq: 2,
        }),
      getContent: async () => new TextEncoder().encode("tool answer"),
    });
    const { result } = renderHook(() => useChat(OPTS), { wrapper: connectedWrapper(client) });

    await act(async () => {
      await result.current.send("use the tool", [], [], ["web-search@1"]);
    });

    await waitFor(() => expect(result.current.thread.messages[1]?.status).toBe("done"));
    expect(result.current.thread.messages[1]).toMatchObject({
      role: "assistant",
      text: "tool answer",
    });
    // Routed through SubmitWorkflow (a single MODEL step carrying the contract), NOT Invoke.
    expect(submitWorkflow).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalled();
    const req = submitted as { steps?: Array<{ toolContract?: Record<string, string> }> };
    expect(req.steps?.[0]?.toolContract).toEqual({ "web-search": "1" });
    expect(getContent).toHaveBeenCalledWith(REF, INSTANCE);
    // The user turn remembers its tools (retry re-fires identically).
    expect(result.current.thread.messages[0]).toMatchObject({ tools: ["web-search@1"] });
  });

  it("fails a tool turn honestly (no hang) when the gateway returns no react chain salt", async () => {
    const { client, submitWorkflow } = makeMockClient({
      // The gateway accepts the workflow but does NOT scope the tool MODEL step
      // agentic — an empty salt means the answer can't be located exactly-once.
      submitWorkflow: async () => ({
        instanceId: INSTANCE,
        reactChainSalt: "",
        recipeFingerprint: "ff".repeat(32),
      }),
    });
    const { result } = renderHook(() => useChat(OPTS), { wrapper: connectedWrapper(client) });

    await act(async () => {
      await result.current.send("use the tool", [], [], ["web-search@1"]);
    });

    await waitFor(() => expect(result.current.thread.messages[1]?.status).toBe("failed"));
    expect(result.current.thread.messages[1]?.error?.title).toMatch(/not supported here/i);
    expect(submitWorkflow).toHaveBeenCalledTimes(1);
    // No hang: the turn settled to failed, so the composer is not stuck busy.
    expect(result.current.busy).toBe(false);
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
