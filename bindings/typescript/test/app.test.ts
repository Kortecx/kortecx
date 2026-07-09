/**
 * POC-4 App-authoring SDK tests (TypeScript).
 *
 * - builder: `app().blueprint(flow()...).toEnvelope()` produces the canonical
 *   `kortecx.app/v1` shape; pending bodies are rejected by `toEnvelope` (use `save`);
 *   a referenced body never inlines (secret-leak).
 * - golden parity (the cross-surface gate): every committed canonical envelope in
 *   `tests/golden/apps/corpus.json` round-trips through THIS SDK's canonicalizer
 *   byte-identically (matches the Rust `kx-app` + the Python SDK).
 * - server-backed (a real `kx serve`): save → list → get → run round-trips.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { afterEach, describe, expect, it } from "vitest";
import { KxClient, Reach, app, canonicalJson, flow, minimalAppEnvelope } from "../src/node.js";
import { devServer, stopAllServers } from "./fixtures/serve.js";

const CORPUS_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "..",
  "tests",
  "golden",
  "apps",
  "corpus.json",
);
const corpus: { name: string; canonical: string }[] = JSON.parse(
  readFileSync(CORPUS_PATH, "utf-8"),
);

afterEach(async () => {
  await stopAllServers();
});

describe("App builder (no server)", () => {
  it("assembles the canonical envelope shape", () => {
    const a = app("Echo Demo")
      .blueprint(flow().agent("Use the echo tool.", { tools: ["mcp-echo/echo"] }))
      .steer({ maxTurns: 8, maxToolCalls: 6 })
      .tags("demo")
      .describe("fires echo");
    const env = a.toEnvelope();
    expect(env.schema).toBe("kortecx.app/v1");
    expect(env.name).toBe("Echo Demo");
    expect((env.steering_config as Record<string, unknown>).guards).toEqual({
      max_turns: 8,
      max_tool_calls: 6,
    });
    expect("references" in env).toBe(false); // empty rails omitted
    // canonicalizes + round-trips.
    expect(JSON.parse(canonicalJson(env))).toEqual(env);
  });

  it("useTool dual-writes the wish and the display rail", () => {
    // useTool records BOTH the display ref (references.tools) AND the wish the server
    // actually consumes (steering_config.tools.requested_grants).
    const env = app("x")
      .blueprint(flow().agent("go"))
      .useTool("mcp-echo/echo")
      .useTool("retrieve", "2")
      .toEnvelope();
    expect((env.references as Record<string, unknown>).tools).toEqual([
      { tool_id: "mcp-echo/echo", tool_version: "1" },
      { tool_id: "retrieve", tool_version: "2" },
    ]);
    expect((env.steering_config as Record<string, unknown>).tools).toEqual({
      requested_grants: { "mcp-echo/echo": "1", retrieve: "2" },
    });
  });

  it("reach: default omitted, inherit_principal emitted, invalid throws", () => {
    const def = app("x")
      .blueprint(flow().agent("go"))
      .steer({ requestedGrants: { e: "1" } })
      .toEnvelope();
    expect(
      (def.steering_config as Record<string, { reach?: string }>).tools?.reach,
    ).toBeUndefined();

    const inh = app("x")
      .blueprint(flow().agent("go"))
      .steer({ reach: Reach.InheritPrincipal })
      .toEnvelope();
    expect((inh.steering_config as Record<string, unknown>).tools).toEqual({
      reach: "inherit_principal",
    });

    expect(() => app("x").blueprint(flow().agent("go")).steer({ reach: "everything" })).toThrow();
  });

  it("rejects toEnvelope with a pending body upload", () => {
    const a = app("x")
      .blueprint(flow().step({ topic: "hi" }))
      .rule("no-pii", { body: "secret" });
    expect(() => a.toEnvelope()).toThrow();
  });

  it("a by-ref artifact never inlines a body", () => {
    const ref = "a".repeat(64);
    const a = app("x")
      .blueprint(flow().step({ topic: "hi" }))
      .rule("policy", { ref });
    const canon = canonicalJson(a.toEnvelope());
    expect(canon).toContain(ref);
    expect(canon).not.toContain("secret");
  });

  it("dataset()/rag() populate references.datasets (RAG-on-App)", () => {
    const env = app("analyst")
      .blueprint(flow().agent("Answer grounded."))
      .dataset("research")
      .rag("archive", { casRefs: ["c".repeat(64)] })
      .toEnvelope();
    const datasets = (env.references as { datasets: unknown[] }).datasets;
    expect(datasets[0]).toEqual({ dataset_ref: "research" }); // no casRefs ⇒ omitted
    expect(datasets[1]).toEqual({ dataset_ref: "archive", cas_refs: ["c".repeat(64)] });
    // canonicalizes + round-trips.
    expect(JSON.parse(canonicalJson(env))).toEqual(env);
  });

  it("dataset() rejects a non-hex casRef", () => {
    expect(() =>
      app("x")
        .blueprint(flow().step({ topic: "hi" }))
        .dataset("d", { casRefs: ["not-hex"] }),
    ).toThrow();
  });

  it("minimalAppEnvelope produces a valid canonical single-step envelope (POC-5a)", () => {
    const env = minimalAppEnvelope("PDF Summarizer", "Summarize uploaded PDFs", {
      model: "gemma-4",
    });
    expect(env.schema).toBe("kortecx.app/v1");
    expect(env.name).toBe("PDF Summarizer");
    expect(env.description).toBe("Summarize uploaded PDFs");
    expect((env.steering_config as Record<string, unknown>).model).toEqual({
      model_route: "gemma-4",
    });
    // a non-empty blueprint (a single agentic step) + canonical round-trip.
    expect(env.blueprint).toBeTruthy();
    expect(JSON.parse(canonicalJson(env))).toEqual(env);
  });
});

describe("App golden corpus parity (the cross-surface byte-shape gate)", () => {
  for (const c of corpus) {
    it(`round-trips ${c.name} byte-identically`, () => {
      expect(canonicalJson(JSON.parse(c.canonical))).toBe(c.canonical);
    });
  }
  it("covers the required shapes", () => {
    const names = new Set(corpus.map((c) => c.name));
    for (const want of ["minimal", "agentic", "full", "grounded", "reach"])
      expect(names.has(want)).toBe(true);
  });
});

describe("App catalog over a real serve", () => {
  it("saves, lists, gets, and runs", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    // A model-free PURE blueprint so the run reaches Committed without a model.
    const a = app("Pure Demo")
      .blueprint(flow().step({ topic: "kortecx" }))
      .describe("pure");
    const saved = await a.save({ client: kx });
    expect(saved.deduplicated).toBe(false);
    expect(saved.handle).toBe("apps/local/pure-demo");

    const apps = await kx.listApps();
    expect(apps.some((x) => x.handle === "apps/local/pure-demo" && x.name === "Pure Demo")).toBe(
      true,
    );

    const stored = await kx.getApp("apps/local/pure-demo");
    expect(stored).not.toBeNull();
    expect((stored?.envelope as Record<string, unknown>).name).toBe("Pure Demo");
    expect(stored?.summary.stepCount).toBe(1);

    // identical re-save dedups.
    const again = await a.save({ client: kx });
    expect(again.deduplicated).toBe(true);

    // run compiles the blueprint and runs it (model-free pure step commits).
    const result = await kx.runApp("apps/local/pure-demo", { wait: true, timeoutMs: 60_000 });
    expect(result).toBeDefined();
  });

  it("a missing App is null", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    expect(await kx.getApp("apps/local/nope")).toBeNull();
  });

  it("save uploads a pending body to a ref", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const a = app("With Rule")
      .blueprint(flow().step({ topic: "hi" }))
      .rule("no-pii", { body: "Never reveal personal data." });
    await a.save({ client: kx });
    const stored = await kx.getApp("apps/local/with-rule");
    if (stored === null) throw new Error("expected the saved App");
    const refs = stored.envelope.references as { rules: { content_ref: string }[] };
    expect(refs.rules[0]?.content_ref ?? "").toHaveLength(64);
    expect(JSON.stringify(stored.envelope)).not.toContain("Never reveal");
  });
});
