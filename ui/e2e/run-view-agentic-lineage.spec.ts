/**
 * The run view tells the truth about an agent run — against a REAL model.
 *
 * The defect this pins: a react/agentic run's turn Motes are registered EDGE-FREE (the
 * coordinator declares no parents, because declaring them would move the canonical
 * digest), so the graph's undirected walk over `parents[]` reached exactly one turn and
 * its observation. An N-turn `kx agent run` rendered as 2 nodes; `kx chat --tools`
 * rendered as 1. The chain was durable the whole time — off-DAG, in the ReactRound
 * facts — and the Timeline tab already read it. Graph and Table did not.
 *
 * WHY THE ASSERTIONS ARE RELATIONAL, NEVER A COUNT. A turn is a model sample: how many
 * turns a 12B takes is the model's behaviour, not the console's, and the chain folds as
 * disjoint stars, so a chain-sized count would assert something we did not create.
 * What we assert is the RELATIONSHIP the fix establishes — turn 0 AND the answer turn
 * are on the same graph, scoped to this run — plus a lower bound, plus a DECOY control.
 *
 * The decoy is load-bearing. After the fix, an anchor that MISSES leaves the projection
 * unscoped, and the graph then renders the whole journal — which sails past any lower
 * bound. Two things separate "the chain was folded together" from "the scope broke":
 * the unscoped notice must be absent, and an unrelated run's Mote must NOT be present.
 *
 * OPT-IN, and loud once opted in. CI has no Ollama, so a spec that failed there would be
 * a lie about the console. `test.skip` therefore gates on the env var ONLY — every other
 * precondition below is an assertion that FAILS with its own message, because a skipped
 * arm and a passing one are indistinguishable in a green summary.
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { buildAgentTurnRequest } from "../src/kx/use-chat";
import { connectConsole, gotoRunHistory } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

/** The three ids a submit hands back. `Run` (no `wait`) carries all of them. */
interface RunHandle {
  readonly instanceId: string;
  readonly reactChainSalt: string;
  readonly terminalMoteId: string;
}

const LIVE = process.env.KX_RUNVIEW_LIVE === "1";
const OLLAMA = process.env.KX_OLLAMA_URL ?? "http://127.0.0.1:11434";
const MODEL = process.env.KX_SERVE_OLLAMA_MODELS ?? "gemma4:12b";
const EMBEDDER = "embeddinggemma:latest";

test.skip(
  !LIVE,
  "live run-view lineage: set KX_RUNVIEW_LIVE=1 and KX_MODEL_BIN=<release-feature-set kx>",
);
test.describe.configure({ mode: "serial" });
test.setTimeout(600_000); // a multi-turn 12B chain is minutes, not the 120s default

/** Fail — never skip — if the models this run's claims depend on are not served. */
async function assertOllamaModels(): Promise<void> {
  const res = await fetch(`${OLLAMA}/api/tags`);
  expect(res.ok, `${OLLAMA}/api/tags did not answer — is Ollama running?`).toBe(true);
  const body = (await res.json()) as { models?: { name: string }[] };
  const names = (body.models ?? []).map((m) => m.name);
  for (const want of [MODEL, EMBEDDER]) {
    expect(names, `Ollama does not serve '${want}'; it has: ${names.join(", ")}`).toContain(want);
  }
}

interface Chain {
  readonly instanceId: string;
  readonly turn0: string;
  readonly answerTurn: string;
  readonly turns: number;
}

/** Drive a real agentic run and read its chain back out of the durable turn record. */
async function driveAgentRun(client: KxClient, goal: string): Promise<Chain> {
  const handle = (await client.invoke("kx/recipes/react", {
    instruction: goal,
    max_turns: 6,
    max_tool_calls: 4,
  })) as RunHandle;
  // The salt is the chain key. For a react Invoke it names the seed the coordinator
  // validates and then DISCARDS, which is exactly why the run view needs a fallback.
  const salt = handle.reactChainSalt;
  expect(salt, "the gateway returned no chain key for a react Invoke").toBeTruthy();

  const deadline = Date.now() + 480_000;
  let rows: Awaited<ReturnType<typeof client.listReactTurns>>["turns"] = [];
  while (Date.now() < deadline) {
    rows = (await client.listReactTurns({ instanceId: handle.instanceId, stepSalt: salt })).turns;
    if (rows.some((r) => r.branch === "answer" || r.branch === "dead_lettered")) {
      break;
    }
    await new Promise((r) => setTimeout(r, 3_000));
  }

  const answer = rows.find((r) => r.branch === "answer");
  // A 12B that answers in one turn, or dead-letters, is a REAL outcome — report it as a
  // failed precondition rather than retrying until it looks like a pass.
  expect(
    answer,
    `the chain never settled on an answer; branches: ${rows.map((r) => r.branch).join(",")}`,
  ).toBeTruthy();
  const distinct = new Set(rows.map((r) => r.turn));
  expect(
    distinct.size,
    `the model answered in ${distinct.size} turn(s) — this proof needs a multi-turn chain`,
  ).toBeGreaterThan(1);

  // The LOWEST-numbered turn is the admitted turn-0 — the only anchor that resolves
  // for a react Invoke, whose chain key names a Mote the coordinator never admits.
  const sorted = [...rows].sort((a, b) => a.turn - b.turn);
  const turn0 = sorted[0];
  expect(turn0, "the chain returned no rows to anchor on").toBeTruthy();
  return {
    instanceId: handle.instanceId,
    turn0: turn0?.turnMoteId ?? "",
    answerTurn: answer?.turnMoteId ?? "",
    turns: distinct.size,
  };
}

/**
 * Open a run the way the console itself does — seed the history record and click the
 * production link. A hand-typed URL cannot work: a reload drops the in-memory
 * connection and bounces to the connect gate.
 */
async function openRun(
  page: import("@playwright/test").Page,
  gw: Gateway,
  anchors: { instanceId: string; terminalMoteId: string; reactChainSalt?: string },
): Promise<void> {
  await page.addInitScript(
    ([ep, rec]) => {
      localStorage.setItem(`kortecx.ui.runs:${ep}`, JSON.stringify([rec]));
    },
    [
      gw.endpoint,
      {
        instanceId: anchors.instanceId,
        terminalMoteId: anchors.terminalMoteId,
        // Seed BOTH anchors, exactly as the console's own `recordRun` does from a
        // RunHandle. For the agentic-launch shape the SALT is the admitted launch Mote
        // and is what resolves; passing only the terminal leaves the run unscoped.
        reactChainSalt: anchors.reactChainSalt ?? null,
        recipeFingerprint: null,
        handle: "kx/recipes/react",
        startedAt: Date.now(),
        args: null,
      },
    ] as const,
  );
  await connectConsole(page, gw);
  await gotoRunHistory(page);
  await page.getByTestId("run-open-full").first().click();
  await expect(page).toHaveURL(/\/workflows\/[0-9a-f]{32}/);
}

test("the graph and table show the whole agent chain, not one turn", async ({ page }, testInfo) => {
  await assertOllamaModels();
  const gw = await spawnGateway({ model: true, console: true, corsOrigin: SPA_ORIGIN });
  try {
    const client = new KxClient(gw.endpoint);
    const models = await client.listModels();
    expect(
      models.length,
      "the serve resolved NO model — every assertion below would fail for the wrong reason",
    ).toBeGreaterThan(0);

    // A second, unrelated run. Its terminal Mote must NOT appear on the first run's
    // graph — that is what separates "the chain was folded" from "the scope broke".
    const decoy = (await client.invoke("kx/recipes/echo", { topic: "decoy" })) as RunHandle;
    const decoyMote = decoy.terminalMoteId;
    expect(decoyMote, "the decoy run returned no anchor").toBeTruthy();

    const chain = await driveAgentRun(
      client,
      "Use your echo tool twice: first echo the word alpha, then echo the word beta. Then tell me both words.",
    );
    testInfo.annotations.push({ type: "chain", description: `${chain.turns} turns` });

    await openRun(page, gw, { instanceId: chain.instanceId, terminalMoteId: chain.turn0 });

    // ⚠ FIRST, and before any count: an anchor that missed leaves the fold UNSCOPED and
    // the graph renders the journal. Without this, the bound below means nothing.
    await expect(page.getByTestId("run-unscoped-notice")).toHaveCount(0);

    await expect(page.getByTestId("mote-dag")).toBeVisible();
    await expect
      .poll(() => page.getByTestId("mote-node").count(), { timeout: 120_000 })
      .toBeGreaterThan(2);

    // The RELATIONSHIP the fix establishes: the run's first turn and the turn that
    // answered are both on the graph. Before the fix the answer turn never was.
    expect(chain.answerTurn).not.toBe(chain.turn0);
    for (const mote of [chain.turn0, chain.answerTurn]) {
      await expect(page.locator(`[data-testid="mote-node"][data-mote="${mote}"]`)).toHaveCount(1);
    }
    // The chain is DRAWN, not merely present: the derived turn-order links render and
    // the canvas says they were derived rather than read off a parent.
    await expect(page.getByTestId("dag-derived-legend")).toBeVisible();
    // The disjointness control.
    await expect(page.locator(`[data-testid="mote-node"][data-mote="${decoyMote}"]`)).toHaveCount(
      0,
    );

    // The TABLE reads the same widened fold — it needed no code of its own.
    await page.getByTestId("run-tab-table").click();
    await expect.poll(() => page.getByTestId("mote-row").count()).toBeGreaterThan(2);
    await expect(page.locator(`[data-testid="mote-row"][data-mote="${decoyMote}"]`)).toHaveCount(0);

    // Both themes, with screenshot evidence. Toggle with no drawer open — an open
    // scrim occludes the navbar control.
    await page.getByTestId("run-tab-graph").click();
    for (const theme of ["light", "dark"] as const) {
      if ((await page.locator("html").getAttribute("data-theme")) !== theme) {
        await page.getByTestId("theme-toggle").click();
      }
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.getByTestId("dag-derived-legend")).toBeVisible();
      await expect(
        page.locator(`[data-testid="mote-node"][data-mote="${chain.answerTurn}"]`),
      ).toHaveCount(1);
      await testInfo.attach(`agent-chain-${theme}`, {
        body: await page.screenshot({ fullPage: true }),
        contentType: "image/png",
      });
    }
  } finally {
    gw.stop();
  }
});

test("a tool-attached chat turn is more than a single node", async ({ page }, testInfo) => {
  await assertOllamaModels();
  const gw = await spawnGateway({ model: true, console: true, corsOrigin: SPA_ORIGIN });
  try {
    const client = new KxClient(gw.endpoint);
    expect((await client.listModels()).length, "the serve resolved no model").toBeGreaterThan(0);

    // The `kx chat --tools` shape: ONE tool-granted agentic step, submitted as a
    // workflow. Here the chain key IS an admitted Mote (the launch), and the loop's
    // answer commits onto it — so the launch alone was the entire graph.
    // Built with the console's OWN request builder, so this proves the shape the
    // product submits rather than one written for the test.
    const handle = (await client.submitWorkflow(
      buildAgentTurnRequest(
        "Echo the word parity using your tool, then tell me what it echoed.",
        { "mcp-echo/echo": "1" },
        6,
        4,
        undefined,
        [],
      ),
    )) as RunHandle;
    const salt = handle.reactChainSalt;
    expect(salt, "no chain key for a tool-attached turn").toBeTruthy();

    const deadline = Date.now() + 480_000;
    let settled = false;
    while (Date.now() < deadline && !settled) {
      const { turns } = await client.listReactTurns({
        instanceId: handle.instanceId,
        stepSalt: salt,
      });
      settled = turns.some((r) => r.branch === "answer" || r.branch === "dead_lettered");
      if (!settled) {
        await new Promise((r) => setTimeout(r, 3_000));
      }
    }
    expect(settled, "the tool-attached turn never settled").toBe(true);

    await openRun(page, gw, {
      instanceId: handle.instanceId,
      terminalMoteId: handle.terminalMoteId,
      reactChainSalt: salt,
    });
    await expect(page.getByTestId("run-unscoped-notice")).toHaveCount(0);
    await expect(page.getByTestId("mote-dag")).toBeVisible();
    await expect
      .poll(() => page.getByTestId("mote-node").count(), { timeout: 120_000 })
      .toBeGreaterThan(1);
    testInfo.annotations.push({
      type: "nodes",
      description: `${await page.getByTestId("mote-node").count()}`,
    });
  } finally {
    gw.stop();
  }
});
