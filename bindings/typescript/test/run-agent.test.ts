/**
 * runAgent — the embeddable agent-runner (PR-9c-1). Pure unit tests (no server).
 *
 * A thin wrapper over `invoke("kx/recipes/react")` + `listReactTurns`; the tests
 * stub the client and assert the assembled `AgentResult` (answer + audited actions),
 * the `wait: false` Run path, prompt-folding of `inputs`, context pass-through, and
 * the zero-config default-client resolution.
 */

import { describe, expect, it } from "vitest";
import { AgentResult, AuditedAction, assembleActions } from "../src/agent-result.js";
import { setDefaultClient } from "../src/default-client.js";
import type { KxClient } from "../src/node.js";
import { ReactTurn } from "../src/react.js";
import { runAgent } from "../src/run-agent.js";
import { Result, Run } from "../src/run.js";

function turn(t: number, branch: string, toolId = "", toolVersion = ""): ReactTurn {
  return new ReactTurn(t, "aa", "bb", "m", branch, toolId, toolVersion, 8, 6, t, "");
}

interface InvokeCall {
  handle: string;
  args: Record<string, unknown>;
  opts: { wait?: boolean; context?: readonly string[] };
}

/** Records the invoke call + returns a canned Result + react-turn page. */
class FakeClient {
  invokeCalls: InvokeCall[] = [];
  constructor(
    private readonly turns: ReactTurn[] = [],
    private readonly payload: Uint8Array | null = new TextEncoder().encode("the answer"),
  ) {}

  async invoke(
    handle: string,
    args: Record<string, unknown>,
    opts: { wait?: boolean; context?: readonly string[]; timeoutMs?: number } = {},
  ): Promise<Run | Result> {
    this.invokeCalls.push({ handle, args, opts });
    if (!opts.wait) {
      return new Run(
        this as unknown as KxClient,
        new Uint8Array(16).fill(1),
        new Uint8Array(32).fill(2),
        new Uint8Array(0),
      );
    }
    return new Result(
      "abababababababab",
      "cd".repeat(32),
      "COMMITTED",
      "ef".repeat(32),
      this.payload,
    );
  }

  async listReactTurns(
    _opts: { instanceId?: string } = {},
  ): Promise<{ turns: ReactTurn[]; hasMore: boolean }> {
    return { turns: this.turns, hasMore: false };
  }
}

describe("assembleActions", () => {
  it("keeps only tool turns, sorted by turn", () => {
    const actions = assembleActions([
      turn(2, "tool", "fs-list", "1"),
      turn(0, "pending"),
      turn(1, "tool", "mcp-echo/echo", "1"),
      turn(3, "answer"),
    ]);
    expect(actions.map((a) => a.turn)).toEqual([1, 2]);
    expect(actions[0]).toEqual(new AuditedAction("mcp-echo/echo", "1", 1));
  });
});

describe("runAgent", () => {
  it("assembles the answer + audited actions", async () => {
    const fc = new FakeClient(
      [turn(0, "tool", "mcp-echo/echo", "1"), turn(1, "answer")],
      new TextEncoder().encode("pong"),
    );
    const out = (await runAgent({
      goal: "echo pong",
      client: fc as unknown as KxClient,
    })) as AgentResult;
    expect(out).toBeInstanceOf(AgentResult);
    expect(out.answer).toBe("pong");
    expect(out.actions.map((a) => a.toolId)).toEqual(["mcp-echo/echo"]);
    expect(out.runHandle).toBe(out.instanceId);
    expect(out.ok).toBe(true);
    const call = fc.invokeCalls[0];
    expect(call?.handle).toBe("kx/recipes/react");
    expect(call?.args.max_turns).toBe(8);
    expect(call?.args.max_tool_calls).toBe(20);
    expect(call?.args.instruction).toBe("echo pong");
  });

  it("returns a Run with wait:false", async () => {
    const fc = new FakeClient();
    const out = await runAgent({ goal: "do it", wait: false, client: fc as unknown as KxClient });
    expect(out).toBeInstanceOf(Run);
    expect(fc.invokeCalls[0]?.opts.wait).toBe(false);
  });

  it("folds inputs into the prompt", async () => {
    const fc = new FakeClient([turn(0, "answer")]);
    await runAgent({
      goal: "summarize",
      inputs: { url: "x", lang: "en" },
      client: fc as unknown as KxClient,
    });
    const instr = fc.invokeCalls[0]?.args.instruction as string;
    expect(instr.startsWith("summarize")).toBe(true);
    expect(instr).toContain("- url: x");
    expect(instr).toContain("- lang: en");
  });

  it("passes context bundles through", async () => {
    const fc = new FakeClient([turn(0, "answer")]);
    await runAgent({ goal: "g", context: ["team/ctx/spec"], client: fc as unknown as KxClient });
    expect(fc.invokeCalls[0]?.opts.context).toEqual(["team/ctx/spec"]);
  });

  it("AgentResult.json() is the snake_case shape", () => {
    const r = new AgentResult(
      "hi",
      new TextEncoder().encode("hi"),
      [new AuditedAction("mcp-echo/echo", "1", 0)],
      "ab",
      "ab",
    );
    expect(r.json()).toEqual({
      instance_id: "ab",
      run_handle: "ab",
      answer: "hi",
      actions: [{ tool_id: "mcp-echo/echo", tool_version: "1", turn: 0, call_index: 0 }],
    });
  });

  it("uses the registry default client when none is passed", async () => {
    const fc = new FakeClient([turn(0, "answer")], new TextEncoder().encode("ok"));
    setDefaultClient(fc as unknown as KxClient);
    try {
      const out = (await runAgent({ goal: "g" })) as AgentResult;
      expect(out.answer).toBe("ok");
    } finally {
      setDefaultClient(undefined);
    }
  });
});
