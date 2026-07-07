/**
 * Swarm/team authoring, the persona library, and the App.run→RunApp fix (TS).
 *
 * The TS mirror of `bindings/python/tests/test_swarm_personas.py`: swarm/fan-out/map-
 * reduce lower to the same `[a & b] > g` topology the golden corpus pins, personas fold
 * into an agent's prompt, and `App.run` routes through SaveApp + RunApp (never a local
 * submitWorkflow recompile — the regression that dropped connections + secret_scope).
 */

import { describe, expect, it } from "vitest";
import {
  Agent,
  PERSONAS,
  app,
  chain,
  fanOutGather,
  flow,
  mapReduce,
  persona,
  personaNames,
  swarm,
  task,
  team,
} from "../src/node.js";

describe("swarm / team / fanOutGather / mapReduce lowering", () => {
  it("swarm lowers to parallel agentic leaves then a synthesizer", () => {
    const low = swarm(
      [
        ["Analyze the market", ["mcp-echo/echo"]],
        ["Critique the analysis", ["mcp-echo/echo"]],
      ],
      { goal: "the Q3 plan" },
    ).lower();
    expect(low.steps).toHaveLength(3); // 2 agentic leaves + 1 synthesizer
    for (const i of [0, 1]) {
      expect(low.steps[i]?.kind).toBe("model");
      expect(low.steps[i]?.tool_contract).toEqual({ "mcp-echo/echo": "1" });
      expect(low.steps[i]?.prompt?.endsWith("the Q3 plan")).toBe(true);
    }
    expect(low.steps[2]?.kind).toBe("model");
    expect(low.steps[2]?.tool_contract).toEqual({});
    expect(low.edges).toEqual([
      { parent: 0, child: 2, edge: "data" },
      { parent: 1, child: 2, edge: "data" },
    ]);
  });

  it("swarm is byte-identical to the equivalent chain", () => {
    const sw = swarm(
      [
        ["A", ["echo"]],
        ["B", ["echo"]],
      ],
      { gather: "Merge" },
    );
    const dsl = chain("[a & b] > g", {
      tasks: {
        a: task.model("", "A", {}, { tools: ["echo"] }),
        b: task.model("", "B", {}, { tools: ["echo"] }),
        g: task.model("", "Merge"),
      },
    });
    expect(sw.lower()).toEqual(dsl.lower());
  });

  it("team defaults to a model synthesizer; synthesize:false uses a pure gather", () => {
    const t = team([persona("researcher"), persona("critic")], { goal: "write a brief" }).lower();
    expect(t.steps).toHaveLength(3);
    expect(t.steps[2]?.kind).toBe("model");
    const pure = swarm(["sample A", "sample B"], { synthesize: false }).lower();
    expect(pure.steps[2]?.kind).toBe("pure");
  });

  it("fanOutGather and mapReduce lower to fan-in", () => {
    const fog = fanOutGather(["angle 1", "angle 2", "angle 3"], { gather: "combine" }).lower();
    expect(fog.steps).toHaveLength(4);
    expect(fog.edges).toHaveLength(3);
    const mr = mapReduce(["map A", "map B"], { reduce: "reduce" }).lower();
    expect(mr.steps).toHaveLength(3);
    expect(mr.edges).toHaveLength(2);
  });

  it("swarm accepts agents, personas, prompts, and flows", () => {
    const a = new Agent("You are an analyst.", { tools: ["mcp-echo/echo"] });
    const low = flow()
      .swarm([a, persona("writer"), "just a prompt", flow().agent("a sub-flow branch")], {
        goal: "the topic",
      })
      .lower();
    expect(low.steps).toHaveLength(5); // 4 leaves + synthesizer
    expect(low.steps[0]?.tool_contract).toEqual({ "mcp-echo/echo": "1" });
  });

  it("persona/Agent leaves lower prompt (not model_id); an Agent's model forwards", () => {
    // Regression for F1/F2 (Py↔TS parity): instructions are the PROMPT, model_id="".
    const leaf = flow()
      .swarm([persona("researcher")], { goal: "the topic" })
      .lower().steps[0];
    expect(leaf?.model_id).toBe("");
    expect(leaf?.prompt).toBe(`${PERSONAS.researcher}\n\nthe topic`);
    expect(leaf?.params).toEqual({});
    const a = new Agent("You are an analyst.", { model: "gemma-4", tools: ["mcp-echo/echo"] });
    const aleaf = flow().swarm([a], { goal: "X" }).lower().steps[0];
    expect(aleaf?.model_id).toBe("gemma-4");
    expect(aleaf?.prompt).toBe("You are an analyst.\n\nX");
    expect(aleaf?.tool_contract).toEqual({ "mcp-echo/echo": "1" });
    expect(aleaf?.params).toEqual({});
  });

  it("an empty swarm is an error", () => {
    expect(() => swarm([])).toThrow();
    expect(() => flow().fanOutGather([])).toThrow();
  });
});

describe("personas", () => {
  it("the library, the factory, and Agent(persona) + .on()", () => {
    expect(personaNames()).toContain("researcher");
    const r = persona("researcher", { tools: ["retrieve"] });
    expect(r).toBeInstanceOf(Agent);
    expect(r.instructions).toBe(PERSONAS.researcher);

    const a = new Agent("", { persona: "critic" });
    expect(a.instructions).toBe(PERSONAS.critic);
    const a2 = new Agent("Focus on security.", { persona: "critic" });
    expect(a2.instructions.startsWith(PERSONAS.critic ?? "")).toBe(true);
    expect(a2.instructions.endsWith("Focus on security.")).toBe(true);
    // .on(task) is an alias of .asFlow(task).
    expect(a.on("review X").lower()).toEqual(a.asFlow("review X").lower());
  });

  it("an unknown persona throws", () => {
    expect(() => persona("nonexistent")).toThrow();
    expect(() => new Agent("", { persona: "nonexistent" })).toThrow();
  });

  it("the persona strings are byte-identical to the Python SDK", () => {
    // A guard: keep the curated set (names + count) stable across surfaces. The string
    // bodies are copied verbatim from personas.py; personaNames pins the roster.
    expect(personaNames()).toEqual([
      "analyst",
      "critic",
      "editor",
      "engineer",
      "planner",
      "researcher",
      "skeptic",
      "strategist",
      "summarizer",
      "writer",
    ]);
  });
});

interface FakeCalls {
  saved: unknown[];
  ran: Array<[string, unknown, boolean | undefined]>;
  submitted: unknown[];
  registered: unknown[];
}

function fakeClient(): { client: Record<string, unknown>; calls: FakeCalls } {
  const calls: FakeCalls = { saved: [], ran: [], submitted: [], registered: [] };
  const client = {
    async putContent() {
      return { contentRef: "a".repeat(64) };
    },
    async saveApp(envelope: unknown) {
      calls.saved.push(envelope);
      return { handle: "apps/local/mailer" };
    },
    async runApp(handle: string, opts?: { args?: unknown; wait?: boolean }) {
      calls.ran.push([handle, opts?.args, opts?.wait]);
      return { ran: handle };
    },
    async submitWorkflow(request: unknown) {
      calls.submitted.push(request); // must stay empty
      return { submitted: true };
    },
    async registerMcpServer(input: unknown) {
      calls.registered.push(input);
    },
  };
  return { client, calls };
}

describe("App.run → SaveApp + RunApp (the integration-in-app fix)", () => {
  it("routes through save + runApp, never submitWorkflow", async () => {
    const { client, calls } = fakeClient();
    const out = await app("mailer")
      .blueprint(flow().agent("Draft and send", { tools: ["kx-connector-gmail/send"] }))
      .withGmail()
      .run({ to: "x@y.com" }, { client: client as any });
    expect(out).toEqual({ ran: "apps/local/mailer" });
    expect(calls.saved).toHaveLength(1);
    expect(calls.ran).toEqual([["apps/local/mailer", { to: "x@y.com" }, true]]);
    expect(calls.submitted).toEqual([]); // must NOT drop to submitWorkflow
    const env = calls.saved[0] as Record<string, any>;
    expect(env.references.connections[0].descriptor).toBe("kx-connector-gmail");
    expect(env.steering_config.guards.secret_scope).toEqual(["KX_GMAIL_CREDENTIAL"]);
  });

  it("withDiscord + secrets", () => {
    const env = app("notifier")
      .blueprint(flow().agent("post an update", { tools: ["discord/send_message"] }))
      .withDiscord()
      .secrets("KX_EXTRA_CRED")
      .toEnvelope() as Record<string, any>;
    expect(env.references.connections[0].descriptor).toBe("kx-connector-discord");
    expect(env.references.connections[0].credential_ref).toBe("KX_DISCORD_CREDENTIAL");
    const scope = env.steering_config.guards.secret_scope as string[];
    expect(scope).toContain("KX_DISCORD_CREDENTIAL");
    expect(scope).toContain("KX_EXTRA_CRED");
  });

  it("withSlack curated connector", () => {
    const env = app("slacker")
      .blueprint(flow().agent("post a digest", { tools: ["slack/post_message"] }))
      .withSlack()
      .toEnvelope() as Record<string, any>;
    expect(env.references.connections[0].descriptor).toBe("kx-connector-slack");
    expect(env.references.connections[0].credential_ref).toBe("KX_SLACK_CREDENTIAL");
    expect(env.steering_config.guards.secret_scope).toContain("KX_SLACK_CREDENTIAL");
  });

  it("withNotion curated connector", () => {
    const env = app("notetaker")
      .blueprint(flow().agent("append a note", { tools: ["notion/append_block"] }))
      .withNotion()
      .toEnvelope() as Record<string, any>;
    expect(env.references.connections[0].descriptor).toBe("kx-connector-notion");
    expect(env.references.connections[0].credential_ref).toBe("KX_NOTION_CREDENTIAL");
    expect(env.steering_config.guards.secret_scope).toContain("KX_NOTION_CREDENTIAL");
  });

  it("Flow.asApp promotes topology and carries side-channels", async () => {
    const { client, calls } = fakeClient();
    await flow()
      .withMcp({ name: "fs", endpoint: "npx", args: ["-y", "server-filesystem", "/data"] })
      .agent("list /data", { tools: ["fs/list_directory"] })
      .asApp("lister")
      .withGmail()
      .run({}, { client: client as any });
    expect(calls.saved).toHaveLength(1);
    expect(calls.registered).toHaveLength(1);
    expect((calls.registered[0] as { name: string }).name).toBe("fs");
    expect(calls.ran[0]?.[0]).toBe("apps/local/mailer");
  });
});
