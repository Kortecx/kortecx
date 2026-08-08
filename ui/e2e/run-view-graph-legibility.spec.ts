/**
 * The run graph is LEGIBLE: no node covers another, and the layout box dagre is given
 * is the box the card actually occupies.
 *
 * The defect this pins: `layout.ts` declared `NODE_H = 72` to dagre while the rendered
 * `.dag-node` is a flex column whose content — accent bar, head row, pill row, result
 * preview, anomaly badge — is more than twice that tall. dagre's `ranksep` is measured
 * from the declared box, so consecutive ranks were placed closer together than the cards
 * are tall and the cards overlapped. Latent while an agent run drew two nodes; unmissable
 * once the run view reads the whole chain.
 *
 * ⚠ WHY THE BOX IS MEASURED AND NOT ASSERTED. Pinning today's numbers would re-pin the
 * same defect the first time the card grows a row. What is asserted is the RELATION:
 * the box handed to dagre must be the box the DOM reports for a real card. A future row
 * changes the number and the assertion still holds.
 *
 * ⚠ WHY THE INSTRUMENT ASSERTS ITS OWN PRECONDITIONS FIRST. Two states would make an
 * overlap count of zero meaningless rather than good: no nodes at all, and nodes that
 * reactflow has not measured yet (a 0x0 rect can never intersect anything). Both are
 * checked and FAIL loudly before a single pair is compared, because a vacuous zero and
 * an earned zero are indistinguishable in a green summary.
 *
 * OPT-IN, and loud once opted in. CI has no Ollama, so `test.skip` gates on the env var
 * ONLY; every other precondition below is an assertion.
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoRunHistory } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

const LIVE = process.env.KX_GRAPHVIEW_LIVE === "1";
const OLLAMA = process.env.KX_OLLAMA_URL ?? "http://127.0.0.1:11434";
const MODEL = process.env.KX_SERVE_OLLAMA_MODELS ?? "gemma4:12b";
const EMBEDDER = "embeddinggemma:latest";

test.skip(
  !LIVE,
  "live graph legibility: set KX_GRAPHVIEW_LIVE=1 and KX_MODEL_BIN=<release-feature-set kx>",
);
test.describe.configure({ mode: "serial" });
test.setTimeout(900_000);

interface RunHandle {
  readonly instanceId: string;
  readonly reactChainSalt: string;
  readonly terminalMoteId: string;
}

/** One rendered card, as the browser reports it. */
interface CardBox {
  readonly mote: string;
  /** Screen-space rect (includes reactflow's viewport zoom) — what a user sees. */
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
  /** The UNTRANSFORMED layout box — comparable to the constants dagre is given. */
  readonly offsetW: number;
  readonly offsetH: number;
}

async function assertOllamaModels(): Promise<void> {
  const res = await fetch(`${OLLAMA}/api/tags`);
  expect(res.ok, `${OLLAMA}/api/tags did not answer — is Ollama running?`).toBe(true);
  const body = (await res.json()) as { models?: { name: string }[] };
  const names = (body.models ?? []).map((m) => m.name);
  for (const want of [MODEL, EMBEDDER]) {
    expect(names, `Ollama does not serve '${want}'; it has: ${names.join(", ")}`).toContain(want);
  }
}

/** Drive a real agentic run; return the anchors the run view opens on. */
async function driveAgentRun(
  client: KxClient,
  goal: string,
): Promise<{ instanceId: string; turn0: string; salt: string; turns: number }> {
  const handle = (await client.invoke("kx/recipes/react", {
    instruction: goal,
    max_turns: 6,
    max_tool_calls: 4,
  })) as RunHandle;
  const salt = handle.reactChainSalt;
  expect(salt, "the gateway returned no chain key for a react Invoke").toBeTruthy();

  const deadline = Date.now() + 600_000;
  let rows: Awaited<ReturnType<typeof client.listReactTurns>>["turns"] = [];
  while (Date.now() < deadline) {
    rows = (await client.listReactTurns({ instanceId: handle.instanceId, stepSalt: salt })).turns;
    if (rows.some((r) => r.branch === "answer" || r.branch === "dead_lettered")) {
      break;
    }
    await new Promise((r) => setTimeout(r, 3_000));
  }
  const distinct = new Set(rows.map((r) => r.turn));
  expect(
    distinct.size,
    `the model answered in ${distinct.size} turn(s) — this measurement needs a multi-turn chain`,
  ).toBeGreaterThan(1);
  const sorted = [...rows].sort((a, b) => a.turn - b.turn);
  return {
    instanceId: handle.instanceId,
    turn0: sorted[0]?.turnMoteId ?? "",
    salt,
    turns: distinct.size,
  };
}

async function openRun(
  page: import("@playwright/test").Page,
  gw: Gateway,
  anchors: { instanceId: string; terminalMoteId: string; reactChainSalt: string },
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
        reactChainSalt: anchors.reactChainSalt,
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

/** Read every rendered card's screen rect AND its untransformed layout box. */
async function readCards(page: import("@playwright/test").Page): Promise<CardBox[]> {
  return page.evaluate(() => {
    const out: CardBox[] = [];
    for (const el of Array.from(document.querySelectorAll('[data-testid="mote-node"]'))) {
      const r = el.getBoundingClientRect();
      const h = el as HTMLElement;
      out.push({
        mote: h.dataset.mote ?? "",
        x: r.x,
        y: r.y,
        w: r.width,
        h: r.height,
        offsetW: h.offsetWidth,
        offsetH: h.offsetHeight,
      });
    }
    return out;
  }) as Promise<CardBox[]>;
}

/** Pairwise rect intersection. Overlap is preserved under reactflow's uniform
 *  scale+translate, so a screen-space count is also the flow-space count. */
function overlappingPairs(
  cards: readonly CardBox[],
): { a: string; b: string; dx: number; dy: number }[] {
  const hits: { a: string; b: string; dx: number; dy: number }[] = [];
  for (let i = 0; i < cards.length; i += 1) {
    for (let j = i + 1; j < cards.length; j += 1) {
      const p = cards[i];
      const q = cards[j];
      if (!p || !q) {
        continue;
      }
      const dx = Math.min(p.x + p.w, q.x + q.w) - Math.max(p.x, q.x);
      const dy = Math.min(p.y + p.h, q.y + q.h) - Math.max(p.y, q.y);
      if (dx > 0 && dy > 0) {
        hits.push({ a: p.mote, b: q.mote, dx, dy });
      }
    }
  }
  return hits;
}

/**
 * One line describing the layout, in FLOW units and naming the AXIS each overlap is
 * on. The axis matters: a collision along y is a rank-height fault (the box dagre
 * reserves per rank) and one along x is a node-separation fault, and the two have
 * different causes. Reporting only a pair count leaves the reader to guess.
 */
function describeLayout(label: string, cards: readonly CardBox[]): string {
  const first = cards[0];
  const zoom = first && first.offsetW > 0 ? first.w / first.offsetW : 1;
  const hits = overlappingPairs(cards);
  const pairs = (cards.length * (cards.length - 1)) / 2;
  // The penetration depth is the SMALLER axis: it is the distance that would have to
  // be recovered to separate the pair.
  const vertical = hits.filter((h) => h.dy <= h.dx);
  const px = (v: number) => `${Math.round(v / zoom)}`;
  const worst = hits.length
    ? `worst=${px(Math.max(...hits.map((h) => Math.min(h.dx, h.dy))))}px ` +
      `axis=${vertical.length === hits.length ? "y" : vertical.length === 0 ? "x" : `y×${vertical.length}/x×${hits.length - vertical.length}`}`
    : "worst=— axis=—";
  const tallest = cards.length ? Math.max(...cards.map((c) => c.offsetH)) : 0;
  const widest = cards.length ? Math.max(...cards.map((c) => c.offsetW)) : 0;
  return (
    `${label} nodes=${cards.length} pairs=${pairs} overlapping=${hits.length} ` +
    `card=${widest}x${tallest} (layout px) zoom=${zoom.toFixed(3)} ${worst}`
  );
}

test("a live agent run's graph draws every node clear of every other, in both themes", async ({
  page,
}, testInfo) => {
  await assertOllamaModels();
  const gw = await spawnGateway({ model: true, console: true, corsOrigin: SPA_ORIGIN });
  try {
    const client = new KxClient(gw.endpoint);
    expect(
      (await client.listModels()).length,
      "the serve resolved NO model — every assertion below would fail for the wrong reason",
    ).toBeGreaterThan(0);

    const chain = await driveAgentRun(
      client,
      "Use your echo tool twice: first echo the word alpha, then echo the word beta. Then tell me both words.",
    );
    testInfo.annotations.push({ type: "chain", description: `${chain.turns} turns` });

    // Seed BOTH anchors, exactly as the console's own `recordRun` does. The salt is
    // what scopes the Timeline's `ListReactTurns`; without it that tab falls back to a
    // per-Mote step list and has no turn cards to compare the graph against.
    await openRun(page, gw, {
      instanceId: chain.instanceId,
      terminalMoteId: chain.turn0,
      reactChainSalt: chain.salt,
    });
    await expect(page.getByTestId("run-unscoped-notice")).toHaveCount(0);
    await expect(page.getByTestId("mote-dag")).toBeVisible();
    await expect
      .poll(() => page.getByTestId("mote-node").count(), { timeout: 180_000 })
      .toBeGreaterThan(2);
    // The fitView refit is rAF-driven and runs once every node is MEASURED; read after
    // it settles or the rects are mid-flight.
    await page.waitForTimeout(1_500);
    const nodeCount = (await readCards(page)).length;

    for (const theme of ["light", "dark"] as const) {
      if ((await page.locator("html").getAttribute("data-theme")) !== theme) {
        await page.getByTestId("theme-toggle").click();
      }
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.waitForTimeout(500);

      const cards = await readCards(page);

      // ── PRECONDITION 1: there is something to measure. ────────────────────────
      expect(
        cards.length,
        "no cards on the canvas — an overlap count would be vacuous",
      ).toBeGreaterThan(2);
      // ── PRECONDITION 2: every card is MEASURED. An unmeasured 0x0 rect cannot
      //    intersect anything, so it would read as a clean layout.
      for (const c of cards) {
        expect(
          c.w,
          `card ${c.mote} has zero screen width — reactflow has not measured it`,
        ).toBeGreaterThan(0);
        expect(
          c.h,
          `card ${c.mote} has zero screen height — reactflow has not measured it`,
        ).toBeGreaterThan(0);
        expect(c.offsetH, `card ${c.mote} has no layout height`).toBeGreaterThan(0);
      }

      const report = describeLayout(theme, cards);
      testInfo.annotations.push({ type: "layout", description: report });
      console.log(`[graph-legibility] ${report}`);

      await testInfo.attach(`graph-${theme}`, {
        body: await page.screenshot({ fullPage: true }),
        contentType: "image/png",
      });

      // ── THE MEASUREMENT. ──────────────────────────────────────────────────────
      const hits = overlappingPairs(cards);
      expect(hits.length, `node pairs overlap in ${theme} — ${report}`).toBe(0);
    }

    // ── THE GRAPH AND THE TIMELINE SAY THE SAME WORDS ABOUT THE SAME TURN. ─────
    // Both surfaces render one `ListReactTurns` row; the graph used to show a Mote
    // hash and an nd_class enum where the Timeline printed the turn, the step type
    // and the fired tool. Read BOTH out of the browser and compare, rather than
    // asserting either against a string written here — a literal in this file would
    // pass while the two surfaces disagreed with each other.
    const graphWords = await page.evaluate(() => {
      const out: Record<string, string[]> = {};
      for (const el of Array.from(document.querySelectorAll('[data-testid="mote-node"]'))) {
        const turn = el.querySelector('[data-testid="dag-node-turn"]')?.textContent?.trim();
        if (!turn) {
          continue;
        }
        out[turn] = [
          el.querySelector('[data-testid="dag-node-step"]')?.textContent?.trim() ?? "",
          el.querySelector('[data-testid="dag-node-branch"]')?.textContent?.trim() ?? "",
        ];
      }
      return out;
    });

    await page.getByTestId("run-tab-timeline").click();
    // The lowest turn index is the model's business, not the console's — wait for ANY
    // turn card rather than pinning turn 0.
    await expect(page.locator('[data-testid^="run-turn-"]').first()).toBeVisible({
      timeout: 30_000,
    });
    const timelineWords = await page.evaluate(() => {
      const out: Record<string, string[]> = {};
      for (const el of Array.from(document.querySelectorAll('[data-testid^="run-turn-"]'))) {
        const turn = el.querySelector(".run-timeline__turn")?.textContent?.trim();
        if (!turn) {
          continue;
        }
        out[turn] = [
          el.querySelector(".badge")?.textContent?.trim() ?? "",
          el.querySelector(".run-timeline__branch")?.textContent?.trim() ?? "",
        ];
      }
      return out;
    });

    const shared = Object.keys(graphWords).filter((k) => k in timelineWords);
    testInfo.annotations.push({
      type: "labels",
      description: `graph=${JSON.stringify(graphWords)} timeline=${JSON.stringify(timelineWords)}`,
    });
    console.log(`[graph-legibility] graph  labels: ${JSON.stringify(graphWords)}`);
    console.log(`[graph-legibility] timeline labels: ${JSON.stringify(timelineWords)}`);
    // ⚠ PRECONDITION: a comparison over an empty intersection passes trivially.
    expect(
      shared.length,
      `no turn appears on BOTH surfaces — the comparison would be vacuous (graph=${Object.keys(graphWords)}, timeline=${Object.keys(timelineWords)})`,
    ).toBeGreaterThan(0);
    for (const turn of shared) {
      expect(graphWords[turn], `the graph and the Timeline disagree about ${turn}`).toEqual(
        timelineWords[turn],
      );
    }
    await page.getByTestId("run-tab-graph").click();
    await expect(page.getByTestId("mote-dag")).toBeVisible();
    await page.waitForTimeout(1_000);

    // ── THE GRAPH ANSWERS TO THE SPACE IT HAS. ─────────────────────────────────
    // The same run on a wide, short window and on a narrow, tall one is the same
    // picture at two very different useful shapes. Both must stay legible — and the
    // layout must actually RESPOND, not merely survive: a static layout would also
    // report zero overlaps here while wasting most of the canvas, so the positions
    // are required to differ between the two shapes.
    const shapes = [
      { label: "wide-short", width: 1600, height: 700 },
      { label: "narrow-tall", width: 900, height: 1200 },
    ] as const;
    const signatures = new Map<string, string>();
    // ⚠ The VIEWPORT TRANSFORM, measured separately from the layout. The W3′ review
    // reported it byte-identical across three container sizes and flagged it as an
    // OPEN QUESTION rather than a defect, because the layout signature below moved and
    // that was the only thing under test. The two are different claims: `layout moved`
    // says dagre re-ran; `transform moved` says `fitView` re-ran. A graph can relayout
    // perfectly and still be framed by a stale fit. Recorded whichever way it comes out.
    const transforms = new Map<string, string>();
    for (const shape of shapes) {
      await page.setViewportSize({ width: shape.width, height: shape.height });
      // The refit is rAF-driven off reactflow's own container measurement.
      await page.waitForTimeout(2_500);
      const cards = await readCards(page);
      const report = describeLayout(shape.label, cards);
      testInfo.annotations.push({ type: "responsive", description: report });
      console.log(`[graph-legibility] ${report}`);
      await testInfo.attach(`graph-${shape.label}`, {
        body: await page.screenshot({ fullPage: true }),
        contentType: "image/png",
      });
      expect(cards.length, `no cards at ${shape.label} — the check would be vacuous`).toBe(
        nodeCount,
      );
      expect(
        overlappingPairs(cards).length,
        `node pairs overlap at ${shape.width}x${shape.height} — ${report}`,
      ).toBe(0);
      // Normalised by zoom so this compares LAYOUT, not the fit transform.
      const first = cards[0];
      const zoom = first && first.offsetW > 0 ? first.w / first.offsetW : 1;
      signatures.set(
        shape.label,
        [...cards]
          .sort((a, b) => a.mote.localeCompare(b.mote))
          .map((c) => `${Math.round(c.x / zoom)},${Math.round(c.y / zoom)}`)
          .join(" "),
      );
      // reactflow writes the fit as a `transform` on its own viewport element — this
      // reads what `fitView` actually produced, not what the layout did.
      const t = await page
        .locator(".react-flow__viewport")
        .first()
        .evaluate((el) => getComputedStyle(el).transform);
      transforms.set(shape.label, t);
      testInfo.annotations.push({
        type: "fit-transform",
        description: `${shape.label} (${shape.width}x${shape.height}): ${t}`,
      });
      console.log(`[graph-legibility] fit-transform ${shape.label}: ${t}`);
    }
    expect(
      signatures.get("wide-short"),
      "the layout is identical at 1600x700 and 900x1200 — it is not responding to the viewport",
    ).not.toEqual(signatures.get("narrow-tall"));

    // ⚠⚠ A DEFECT-REPRODUCING ASSERTION. IT IS SUPPOSED TO SAY `toEqual`. INVERT IT
    // TO `.not.toEqual` WHEN THE DEFECT IS FIXED, AND DELETE THIS PARAGRAPH.
    //
    // The prior review flagged, as an open question rather than a claim, that
    // `fitView`'s transform looked byte-identical across container sizes. Measured on a
    // live model-served run, it is:
    //
    //   1600x700  -> matrix(1.15854, 0, 0, 1.15854, 102.561, -0.536585)
    //   900x1200  -> matrix(1.15854, 0, 0, 1.15854, 102.561, -0.536585)
    //
    // …while the LAYOUT above demonstrably did change (the zoom-normalised position
    // signatures differ, which is asserted). So dagre re-runs and the viewport does
    // not: the canvas keeps a frame computed for a container that no longer exists.
    // Two container shapes whose aspect ratios differ by more than 3x cannot honestly
    // produce the same fit.
    //
    // Recorded as a reproduction rather than left as a failing assertion, so the suite
    // states what is true today and goes RED the moment someone fixes it — the same
    // shape as `a_react_warrant_change_conflicts_on_an_already_seeded_state_dir`, which
    // was written to be inverted and duly was.
    expect(
      transforms.get("wide-short"),
      "fitView now produces DIFFERENT transforms at 1600x700 and 900x1200 — the refit " +
        "defect appears to be FIXED. Invert this assertion to `.not.toEqual` and " +
        "delete the paragraph above it.",
    ).toEqual(transforms.get("narrow-tall"));

    // ── THE BOX IS DERIVED FROM THE RENDERED CARD, not from a constant. ─────────
    // Grow the card by more than a row's worth and the layout must follow. This is
    // the acceptance criterion rather than "the overlap count went down": a layout
    // fed hard-coded dimensions passes the count above and fails here, because a
    // constant cannot track a card it does not know grew.
    const beforeGrow = await readCards(page);
    const grownBy = 105;
    const wasTallest = Math.max(...beforeGrow.map((c) => c.offsetH));
    await page.addStyleTag({
      content: `.dag-node { min-height: ${wasTallest + grownBy}px !important; }`,
    });
    // ⚠ PRECONDITION: the mutation must actually reach the card. An injected rule
    // that lost a specificity fight would leave the card its old size, and "still no
    // overlaps" would then be a statement about nothing.
    await expect
      .poll(async () => Math.max(...(await readCards(page)).map((c) => c.offsetH)), {
        timeout: 20_000,
      })
      .toBeGreaterThanOrEqual(wasTallest + grownBy);
    await page.waitForTimeout(2_000); // let the re-measure settle into a relayout

    const afterGrow = await readCards(page);
    const grownReport = describeLayout("grown", afterGrow);
    testInfo.annotations.push({ type: "layout-grown", description: grownReport });
    console.log(`[graph-legibility] ${grownReport}`);
    await testInfo.attach("graph-grown-card", {
      body: await page.screenshot({ fullPage: true }),
      contentType: "image/png",
    });
    expect(
      overlappingPairs(afterGrow).length,
      `the card grew ${wasTallest}px → ${wasTallest + grownBy}px and the layout did not follow — ${grownReport}`,
    ).toBe(0);
  } finally {
    gw.stop();
  }
});
