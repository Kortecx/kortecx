#!/usr/bin/env node
// Three checks Docusaurus's own build cannot make.
//
// (1) ORPHAN PAGES — a page absent from the hand-authored `sidebars.ts` is
//     unreachable by navigation. Docusaurus builds it happily (it is a valid route),
//     so nothing complains. `llm-rerank.md` sat orphaned this way.
//
// (2) ANCHORS ON LINKS THAT GO THROUGH GITHUB.
//
// Docusaurus resolves and checks its own relative links (`onBrokenLinks` /
// `onBrokenAnchors`), but a link written as an absolute
// `https://github.com/Kortecx/kortecx/blob/main/README.md#cli-reference` is just an
// external URL to it — so a heading can be renamed or deleted and every gate stays
// green while the reader lands at the top of a long file with no idea what they were
// promised. That is how `README.md#cli-reference` survived: the README has never had
// a "CLI reference" heading.
//
// This checks that class, and only that class. Relative doc-to-doc anchors stay
// Docusaurus's job (it understands explicit `{#id}` overrides; a second slugifier
// here would disagree with it eventually).
//
// (3) THE README'S BENCHMARK TABLES vs the committed per-engine baselines — the
//     numbers the project leads with. A re-baseline is deliberate, so the tables and
//     the baseline must move together; otherwise the README advertises a score the
//     ratchet no longer holds. Every per-task-binary cell publishes its exact
//     fraction (`pm · N/M`), inverted and re-folded here so the fraction cannot
//     drift from the corpus; graded metrics carry a †/‡ marker instead (a fraction
//     there would be fabrication) — the marker requirement is DERIVED from the
//     corpus, never styled by hand.
//
// (5) EVALUATION.MD'S FAMILY TABLE — the docs-site twin, held to the same baselines
//     by the same checker.
//
// (6) THE README'S DENOMINATOR CHARTS — the anchored mermaid charts' labels, order
//     and bar values, held to the same corpus counts and baselines as the tables.
//
// Usage: node scripts/check-docs.mjs        (from docs/site/)

import { readdir, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
// The ONE declaration of which families are agentic capability and which are not, shared
// with the generator so the chart and its checker cannot disagree about the headline set.
import { NOT_AGENTIC_CAPABILITY, passesFrom } from "./render-bench-chart.mjs";

// Not `import.meta.dirname` — that needs Node 20.11, and this package declares
// `engines.node >= 18`.
const SITE = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPO = resolve(SITE, "..", "..");
const BLOB = /https:\/\/github\.com\/Kortecx\/kortecx\/blob\/[^/]+\/([^)\s#]+\.md)#([^)\s"]+)/g;

/** Every `.md` under `dir`, recursively. */
async function markdownUnder(dir) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await markdownUnder(path)));
    else if (entry.name.endsWith(".md")) out.push(path);
  }
  return out;
}

/** Files whose links we check. */
const SOURCES = [
  ...(await markdownUnder(join(SITE, "docs"))),
  join(REPO, "README.md"),
  join(REPO, "bindings/python/README.md"),
  join(REPO, "bindings/typescript/README.md"),
];

/**
 * GitHub's heading slug: lowercase, drop everything that is not a letter, digit,
 * space, hyphen or underscore, then spaces to hyphens. Deliberately literal —
 * "Workflows, chains & forms" keeps the double hyphen the removed `&` leaves behind,
 * because that is the anchor GitHub actually serves.
 */
function slug(heading) {
  return heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N} _-]/gu, "")
    .replace(/ /g, "-");
}

/** Every anchor a markdown file offers, from its ATX headings. */
async function anchorsOf(path) {
  const text = await readFile(path, "utf8");
  const found = new Set();
  const counts = new Map();
  for (const line of text.split("\n")) {
    const m = /^#{1,6}\s+(.*)$/.exec(line);
    if (!m) continue;
    const base = slug(m[1]);
    if (!base) continue;
    // GitHub disambiguates repeats with -1, -2, … — mirror it so a legitimate
    // link to a duplicated heading is not reported as broken.
    const n = counts.get(base) ?? 0;
    counts.set(base, n + 1);
    found.add(n === 0 ? base : `${base}-${n}`);
  }
  return found;
}

const anchorCache = new Map();
const problems = [];

// (1) Every docs page must be listed in the hand-authored sidebar.
{
  const sidebar = await readFile(join(SITE, "sidebars.ts"), "utf8");
  const docsRoot = join(SITE, "docs");
  for (const path of await markdownUnder(docsRoot)) {
    // The sidebar keys pages by their doc id — the repo convention is the path
    // relative to docs/ without the extension (`chains/python`).
    const id = path.slice(docsRoot.length + 1).replace(/\.md$/, "");
    if (!sidebar.includes(`"${id}"`)) {
      problems.push(
        `sidebars.ts: "${id}" is not listed — the page builds but nothing links to it`,
      );
    }
  }
}

// (3) The README's benchmark table must equal the committed per-engine baselines.
// These are the numbers the project leads with, and a re-baseline is a deliberate act
// — so the table and the baseline it quotes have to move together or the README starts
// advertising a score the ratchet no longer holds.
{
  const readme = await readFile(join(REPO, "README.md"), "utf8");
  const engines = [
    ["Ollama", "crates/kx-eval/corpus/bench-v1/baseline.ollama.json"],
    ["llama.cpp", "crates/kx-eval/corpus/bench-v1/baseline.llamacpp.json"],
  ];
  const loaded = [];
  for (const [name, rel] of engines) {
    const path = join(REPO, rel);
    if (!existsSync(path)) {
      problems.push(`${rel}: missing — the README quotes its numbers`);
      continue;
    }
    const doc = JSON.parse(await readFile(path, "utf8"));
    loaded.push([
      name,
      new Map(doc.gates.map((g) => [g.id, g.per_mille])),
      doc.env,
      new Map((doc.spikes ?? []).map((s) => [s.id, s])),
    ]);
  }

  // The column order is read from the table's own header rather than assumed: comparing
  // a claimed number against the wrong engine's baseline would pass or fail for the
  // wrong reason, and swapping two columns is an easy edit.
  const header = /^\|\s*Family\s*\|[^|]*\|[^|]*\|([^|]*)\|([^|]*)\|/m.exec(readme);
  if (!header) {
    problems.push("README.md: benchmark table header not found — cannot verify its numbers");
  } else {
    const declared = [header[1].trim(), header[2].trim()];
    const expected = engines.map(([name]) => name);
    if (declared.join("|") !== expected.join("|")) {
      problems.push(
        `README.md: benchmark columns read ${declared.join(", ")}; this check compares ` +
          `them as ${expected.join(", ")} — reorder the check or the table`,
      );
    }
  }

  // The family rows are derived from the CORPUS, not from a hand-kept list here. The
  // previous version matched `(tool|react|reach|swarm)` and asserted there were four —
  // so the `script` row it did not name was published, unchecked, for as long as it
  // existed, and adding a family meant remembering to edit a regex in another directory.
  // Reading the corpus makes a new family a check that fails until the README carries it.
  const suitePath = join(REPO, "crates/kx-eval/corpus/bench-v1/suite.json");
  /** @type {Map<string, number>} family → task count */
  const taskCounts = new Map();
  /** @type {Array<Record<string, unknown>>} the corpus tasks, for expectation counts */
  let suiteTasks = [];
  if (!existsSync(suitePath)) {
    problems.push("crates/kx-eval/corpus/bench-v1/suite.json: missing — the README quotes it");
  } else {
    const suite = JSON.parse(await readFile(suitePath, "utf8"));
    suiteTasks = suite.tasks ?? [];
    for (const t of suiteTasks) {
      taskCounts.set(t.family, (taskCounts.get(t.family) ?? 0) + 1);
    }
  }
  const suiteTotal = [...taskCounts.values()].reduce((a, b) => a + b, 0);

  // The fraction machinery. A published `pm · N/M` cell is honest ONLY where the
  // metric is pass/fail per task — task_success everywhere, and injection_resistance
  // (whose denominator is the count of tasks DECLARING an injection expectation, not
  // the family or suite size: the scorer is N/A everywhere else). The graded metrics
  // are per-task fractions averaged, so no task fraction exists for them — a cell
  // carrying one there is a fabrication and fails. The single-task graded metrics
  // (`groundedness`/`memory_quality`, each exercised by exactly one corpus task
  // today) carry `‡`; the multi-task graded ones carry `†`. Both markers are DERIVED
  // from the corpus here, so adding a second RAG or memory task makes a stale `‡`
  // fail until the README stops claiming n=1.
  const nonEmpty = (a) => Array.isArray(a) && a.length > 0;
  const expectCount = (pick) => suiteTasks.filter((t) => pick(t.expect ?? {})).length;
  const injectionTaskCount = expectCount(
    (e) => nonEmpty(e.forbidden_tools) || nonEmpty(e.answer_must_not_contain),
  );
  const groundedTaskCount = expectCount((e) => nonEmpty(e.grounded_in));
  const memoryTaskCount = expectCount((e) => nonEmpty(e.memory_must_recall));
  // The sequence columns apply exactly where a gold call sequence exists.
  const seqTaskCount = expectCount((e) => nonEmpty(e.expected_tools));
  // The pass^k population is corpus data (`flagship: true`), so its denominator is
  // derived here like every other one — changing the flagship set moves this check,
  // the suite digest, and the baseline together.
  const flagshipCount = suiteTasks.filter((t) => t.flagship === true).length;
  // The Success@8 gate's denominator is its QUERY count, which lives beside the probe
  // (RETRIEVAL_PROBES in crates/kx-gateway/tests/eval_bench_real.rs) — the one
  // published denominator not derivable from the corpus, so it is pinned here and the
  // `retrieval_success_at_8@queries` sentinel (1000 ⇔ every query executed) is what
  // keeps a probe-count drift from passing silently.
  const RETRIEVAL_PROBE_COUNT = 10;
  /** @type {Map<string, number>} suite-wide binary metric → its honest denominator */
  const binaryDenominators = new Map([
    ["task_success", suiteTotal],
    ["injection_resistance", injectionTaskCount],
    ["tool_seq_fsa", seqTaskCount],
    ["pass_k4", flagshipCount],
    ["retrieval_success_at_8", RETRIEVAL_PROBE_COUNT],
  ]);
  const CELL = /^(\d+)\s*·\s*(\d+)\/(\d+)$/u;

  /**
   * Check one 5-column family table (`| family | Tasks | prose | engine | engine |`,
   * every engine cell `pm · N/M`) against the corpus counts and both baselines.
   * Returns per-engine pass-count sums and the row order, for the cross-checks.
   */
  function checkFamilyTable(source, rows) {
    const order = [];
    const sums = loaded.map(() => 0);
    const claimed = new Set(rows.map((r) => r[1]));
    for (const family of taskCounts.keys()) {
      const withheld = NOT_AGENTIC_CAPABILITY.has(family);
      if (!withheld && !claimed.has(family)) {
        problems.push(
          `${source}: the benchmark table has no row for the agentic '${family}' family, which ` +
            `the corpus contains — an unpublished agentic family is a measured capability the ` +
            `reader never sees`,
        );
      }
      // The withheld families are authoring/scripting surfaces whose per-family results
      // are captured but not broken out per family here. Publishing one would contradict the
      // scope line and silently change what the headline claims to cover.
      if (withheld && claimed.has(family)) {
        problems.push(
          `${source}: the benchmark table publishes '${family}', which is declared withheld ` +
            `in render-bench-chart.mjs. Either drop the row or remove it from ` +
            `NOT_AGENTIC_CAPABILITY — the table and the declaration must agree`,
        );
      }
    }
    for (const [, family, count, ...cols] of rows) {
      order.push(family);
      const expectedCount = taskCounts.get(family);
      if (expectedCount === undefined) {
        problems.push(`${source}: benchmark table names a '${family}' family the corpus does not have`);
        continue;
      }
      if (Number(count) !== expectedCount) {
        problems.push(`${source}: '${family}' claims ${count} task(s), the corpus has ${expectedCount}`);
      }
      loaded.forEach(([engine, gate], i) => {
        const cell = cols[i].trim();
        const parsed = CELL.exec(cell);
        if (!parsed) {
          problems.push(
            `${source}: '${family}' on ${engine} reads '${cell}' — every family cell is ` +
              `'per-mille · passes/tasks' (task_success is binary per task, so the fraction is exact)`,
          );
          return;
        }
        const [, pm, num, den] = parsed.map(Number);
        const actual = gate.get(`task_success@${family}`);
        if (actual === undefined) {
          problems.push(`baseline.${engine}: no task_success@${family} gate, but ${source} quotes one`);
          return;
        }
        if (pm !== actual) {
          problems.push(`${source}: ${family} on ${engine} reads ${pm}, baseline says ${actual}`);
        }
        if (den !== expectedCount) {
          problems.push(
            `${source}: '${family}' fraction denominator reads ${den} on ${engine}, the corpus has ${expectedCount} task(s)`,
          );
        }
        if (Math.floor((1000 * num) / den) !== pm) {
          problems.push(
            `${source}: '${family}' on ${engine} claims ${num}/${den}, which folds to ` +
              `${Math.floor((1000 * num) / den)} — the table says ${pm}`,
          );
        }
        sums[i] += num;
      });
    }
    return { order, sums };
  }

  // `| **tool** | 6 | …prose… | 1000 | 833 |` — family, task COUNT, prose, then one
  // column per engine in the header's order. The count is checked because it is the
  // denominator: without it a reader cannot tell that 833 means five tasks out of six,
  // and a family of one looks as solid as a family of six.
  const rows = [
    ...readme.matchAll(
      /^\|\s*\*\*([a-z]+)\*\*\s*\|\s*(\d+)\s*\|[^|]*\|([^|]*)\|([^|]*)\|/gm,
    ),
  ];
  const readmeTable = checkFamilyTable("README.md", rows);

  // The SUITE-WIDE numbers. These are the ones a family table can hide: every family can
  // read well while `loop_efficiency` sits far below it, and publishing only the flattering
  // half is the failure this check exists to make impossible.
  // Format: `| `task_success` | 769 · 20/26 | 769 · 20/26 |` for the per-task-binary
  // metrics, `| `loop_efficiency` † | 678 | 939 |` for the graded ones (whose marker
  // is enforced from the corpus — see the fraction machinery above).
  const suiteRows = [
    ...readme.matchAll(/^\|\s*`([a-z_0-9]+)`\s*([†‡]?)\s*\|([^|]*)\|([^|]*)\|/gm),
  ];
  const publishedSuiteGates = new Set();
  for (const [, metric, marker, ...cols] of suiteRows) {
    const isBinary = binaryDenominators.has(metric);
    const singleTask =
      (metric === "groundedness" && groundedTaskCount === 1) ||
      (metric === "context_recall" && groundedTaskCount === 1) ||
      (metric === "memory_quality" && memoryTaskCount === 1);
    loaded.forEach(([engine, gate], i) => {
      const actual = gate.get(metric);
      if (actual === undefined) return; // not a gate row (a metric-definition table)
      publishedSuiteGates.add(metric);
      const cell = cols[i].trim();
      if (isBinary) {
        const parsed = CELL.exec(cell);
        if (!parsed) {
          problems.push(
            `README.md: suite-wide ${metric} on ${engine} reads '${cell}' — a per-task-binary ` +
              `metric publishes 'per-mille · passes/tasks'`,
          );
          return;
        }
        const [, pm, num, den] = parsed.map(Number);
        const expectedDen = binaryDenominators.get(metric);
        if (pm !== actual) {
          problems.push(
            `README.md: suite-wide ${metric} on ${engine} reads ${pm}, baseline says ${actual}`,
          );
        }
        if (den !== expectedDen) {
          problems.push(
            `README.md: ${metric} fraction denominator reads ${den} on ${engine}; the corpus ` +
              `applies the metric to ${expectedDen} task(s)`,
          );
        }
        if (Math.floor((1000 * num) / den) !== pm) {
          problems.push(
            `README.md: ${metric} on ${engine} claims ${num}/${den}, which folds to ` +
              `${Math.floor((1000 * num) / den)} — the table says ${pm}`,
          );
        }
        return;
      }
      // Graded: the cell is a bare per-mille. A fraction here would be fabrication —
      // the score is a per-task fraction AVERAGED, so no pass-count exists.
      if (!/^\d+$/.test(cell)) {
        problems.push(
          `README.md: suite-wide ${metric} on ${engine} reads '${cell}' — a graded metric is a ` +
            `bare per-mille (it is averaged per task, so a pass-count fraction would be fabrication)`,
        );
        return;
      }
      if (Number(cell) !== actual) {
        problems.push(
          `README.md: suite-wide ${metric} on ${engine} reads ${cell}, baseline says ${actual}`,
        );
      }
    });
    // The marker is part of the record: binary rows carry none, graded rows carry
    // `†`, single-task graded rows carry `‡` — derived from the corpus, never styled
    // by hand.
    if (publishedSuiteGates.has(metric)) {
      const required = isBinary ? "" : singleTask ? "‡" : "†";
      if (marker !== required) {
        problems.push(
          `README.md: suite-wide ${metric} carries marker '${marker || "(none)"}' but the corpus ` +
            `requires '${required || "(none)"}' — † is graded, ‡ is graded-with-one-task, binary rows are unmarked`,
        );
      }
    }
  }
  // The family pass-counts must sum to the suite-wide task_success fraction, per
  // engine — an internally-inconsistent hand edit fails even when each row happens
  // to floor-fold correctly.
  {
    const suiteSuccessRow = suiteRows.find((r) => r[1] === "task_success");
    if (suiteSuccessRow) {
      // The published table covers the AGENTIC families only, so the reconciliation is
      // shown + withheld == suite-wide. This is what stops a withheld family from
      // quietly becoming an unmeasured one: if the internal families ever stopped being
      // scored, their contribution would drop and this sum would fail.
      loaded.forEach(([engine, gate], i) => {
        const parsed = CELL.exec(suiteSuccessRow[3 + i].trim());
        if (!parsed) return;
        let withheldPasses = 0;
        for (const family of NOT_AGENTIC_CAPABILITY.keys()) {
          const n = taskCounts.get(family);
          if (n === undefined) continue;
          withheldPasses += passesFrom(gate.get(`task_success@${family}`) ?? 0, n);
        }
        const total = readmeTable.sums[i] + withheldPasses;
        if (total !== Number(parsed[2])) {
          problems.push(
            `README.md: the published family pass-counts sum to ${readmeTable.sums[i]} on ` +
              `${engine}, plus ${withheldPasses} withheld = ${total}, but suite-wide ` +
              `task_success claims ${parsed[2]}/${suiteTotal}`,
          );
        }
      });
    }
  }
  // Every suite-wide gate must appear, including the unflattering ones.
  for (const [engine, gate] of loaded) {
    for (const id of gate.keys()) {
      if (id.includes("@")) continue; // per-family gates live in the family table
      if (!publishedSuiteGates.has(id)) {
        problems.push(
          `README.md: baseline.${engine} carries a suite-wide '${id}' gate the README does ` +
            `not publish — a number omitted from the table is a number chosen not to show`,
        );
      }
    }
  }

  // (4) THE ENVIRONMENT LABEL. A real-model score without the model that produced it is
  //     not a record. The README used to say "Gemma-4-12B on both local engines" while the
  //     two arms ran two DIFFERENT builds, and nothing could catch it because the committed
  //     baseline carried no label at all.
  for (const [engine, , env] of loaded) {
    if (!env) {
      problems.push(
        `baseline.${engine}: no env block — re-capture it (KX_BENCH_UPDATE_BASELINE=1), ` +
          `since the README's environment claims are checked against it`,
      );
      continue;
    }
    if (env.model && !readme.includes(env.model)) {
      problems.push(
        `README.md: does not name '${env.model}', the model baseline.${engine} was captured ` +
          `on — the two engines do not run the same build, and saying so is the point`,
      );
    }
    if (env.task_count && suiteTotal && env.task_count !== suiteTotal) {
      problems.push(
        `baseline.${engine}: captured over ${env.task_count} task(s), the corpus now has ` +
          `${suiteTotal} — re-capture before publishing`,
      );
    }
  }

  // (5) EVALUATION.MD'S FAMILY TABLE — the docs-site twin of the README table, in the
  //     same 5-column `pm · N/M` form (family names backticked there). It was prose
  //     nothing checked; now the shared checker locks it to the same baselines, so the
  //     two tables cannot diverge from the corpus or from each other.
  {
    const evaluation = await readFile(join(SITE, "docs/evaluation.md"), "utf8");
    const evalRows = [
      ...evaluation.matchAll(
        /^\|\s*`([a-z]+)`\s*\|\s*(\d+)\s*\|[^|]*\|([^|]*)\|([^|]*)\|/gm,
      ),
    ];
    if (evalRows.length === 0) {
      problems.push(
        "docs/evaluation.md: no 5-column family table found (| `family` | Tasks | prose | " +
          "Ollama | llama.cpp | with `pm · N/M` cells) — the docs-site twin of the README table",
      );
    } else {
      checkFamilyTable("docs/evaluation.md", evalRows);
    }
  }

  // (6) THE README's TWO-ENGINE COMPARISON CHART — now a GENERATED, COMMITTED SVG.
  //
  //     Mermaid OVERLAYS multiple series and emits no legend, so a shorter bar can hide
  //     entirely behind a taller one and a true 0 is indistinguishable from a family that
  //     was never scored. The grouped chart is therefore rendered to SVG and committed;
  //     GitHub shows committed SVG in markdown but not styled HTML.
  //
  //     A generated image that nothing validates is a SCREENSHOT, and screenshots go stale
  //     without anything failing — the exact class that let the capture-provenance line rot
  //     for two weeks. So each row carries `data-family/-n/-a/-b`, and those attributes are
  //     held to the committed baselines here.
  {
    const pic = /<!--\s*bench-chart:comparison\b[\s\S]*?-->\s*<picture>([\s\S]*?)<\/picture>/.exec(readme);
    if (!pic) {
      problems.push(
        "README.md: no <!-- bench-chart:comparison --> anchored <picture> — regenerate with " +
          "`node docs/site/scripts/render-bench-chart.mjs --write`",
      );
    } else {
      for (const theme of ["light", "dark"]) {
        const rel = `docs/assets/bench-agentic-${theme}.svg`;
        if (!pic[1].includes(rel)) {
          problems.push(`README.md: the comparison <picture> does not reference ${rel}`);
          continue;
        }
        const path = join(REPO, rel);
        if (!existsSync(path)) {
          problems.push(`${rel}: missing — the README's chart points at it`);
          continue;
        }
        const svg = await readFile(path, "utf8");
        const rows = [
          ...svg.matchAll(
            /data-family="([a-z]+)"\s+data-n="(\d+)"\s+data-a="(\d+)"\s+data-b="(\d+)"/g,
          ),
        ];
        if (rows.length === 0) {
          problems.push(
            `${rel}: carries no data-family rows — the chart cannot be checked against the ` +
              `baselines, so it is a screenshot`,
          );
          continue;
        }
        for (const [, family, n, a, b] of rows) {
          const expected = taskCounts.get(family);
          if (expected === undefined) {
            problems.push(`${rel}: plots a '${family}' family the corpus does not have`);
            continue;
          }
          if (Number(n) !== expected) {
            problems.push(`${rel}: '${family}' claims n=${n}, the corpus has ${expected}`);
          }
          if (NOT_AGENTIC_CAPABILITY.has(family)) {
            problems.push(
              `${rel}: plots '${family}', which is declared NON-agentic in ` +
                `render-bench-chart.mjs and must not appear in the headline`,
            );
          }
          [
            ["Ollama", 0, "ollama", a],
            ["llama.cpp", 1, "llamacpp", b],
          ].forEach(([engine, idx, fileKey, plotted]) => {
            const actual = loaded[idx]?.[1]?.get(`task_success@${family}`);
            if (actual !== undefined && Number(plotted) !== actual) {
              problems.push(
                `${rel}: ${engine} value for '${family}' reads ${plotted}, ` +
                  `baseline.${fileKey}.json says ${actual}`,
              );
            }
          });
        }
        // Every agentic family must be PLOTTED, not merely consistent when present.
        const plotted = new Set(rows.map((r) => r[1]));
        for (const family of taskCounts.keys()) {
          if (NOT_AGENTIC_CAPABILITY.has(family)) continue;
          if (!plotted.has(family)) {
            problems.push(`${rel}: the agentic '${family}' family is missing from the chart`);
          }
        }
      }

      // WITHHELD ≠ HIDDEN — the disclosed aggregate, checked against the baselines.
      let wTasks = 0;
      const wPasses = loaded.map(() => 0);
      for (const family of NOT_AGENTIC_CAPABILITY.keys()) {
        const n = taskCounts.get(family);
        if (n === undefined) continue;
        wTasks += n;
        loaded.forEach(([, gate], i) => {
          wPasses[i] += passesFrom(gate.get(`task_success@${family}`) ?? 0, n);
        });
      }
      if (wTasks > 0) {
        if (!new RegExp(String.raw`\*\*${wTasks} authoring and scripting tasks\*\*`).test(readme)) {
          problems.push(
            `README.md: the benchmark scope line must state that **${wTasks} authoring and ` +
              `scripting tasks** are withheld — a filtered table beside an unfiltered ` +
              `suite-wide figure does not add up, and the reader cannot tell why`,
          );
        }
        const disclosed = new RegExp(
          String.raw`\(${wPasses[0]} and\s+${wPasses[1]} passes respectively\)`,
        );
        if (!disclosed.test(readme.replace(/\s+/g, " "))) {
          problems.push(
            `README.md: the withheld families contributed ${wPasses[0]} and ${wPasses[1]} ` +
              `passes per the committed baselines; the scope line must say so. Regenerate ` +
              `with \`node docs/site/scripts/render-bench-chart.mjs --write\``,
          );
        }
      }
    }
  }

  // (6b) THE CAPTURE PROVENANCE. The commit and date a published number was measured at.
  //      This check exists because its absence had a consequence: the README named a
  //      capture from `5a67e740` / 2026-07-31 long after the committed baselines moved to
  //      `0f16840f` / 2026-08-03, and every NUMBER on the page validated while the
  //      sentence describing them was two weeks stale. A gate that checks the figures and
  //      not the claim about the figures is not checking provenance at all.
  {
    const sha = (loaded[0]?.[2]?.git_sha ?? "").slice(0, 12);
    if (sha && !readme.includes(sha)) {
      problems.push(
        `README.md: the benchmark section does not name the capture commit '${sha}' that ` +
          `baseline.ollama.json records. Regenerate with ` +
          `\`node docs/site/scripts/render-bench-chart.mjs --write\`.`,
      );
    }
    const stale = readme.match(/\*\*Captured \d{4}-\d{2}-\d{2}\*\*\s*\(`([0-9a-f]{6,})`/);
    if (stale && sha && !stale[1].startsWith(sha.slice(0, 6))) {
      problems.push(
        `README.md: a hand-written "Captured …" line names '${stale[1]}' but the committed ` +
          `baseline was captured at '${sha}'`,
      );
    }
  }

  // (7) THE PERFORMANCE TABLE — the published absolutes (tokens, latency), held to the
  //     Spikes committed in the same baselines the gates come from. Spikes are never
  //     gated (a slower host moves them and that is not a regression), but a published
  //     absolute with no committed source is a number nobody can check — so the
  //     baseline is where the docs' copy is verified, exactly as the env label is.
  //     Every entry below must have a row; a cell is `<integer> <unit>` where the
  //     capture recorded the spike and the em-dash `—` where it did not (an absent
  //     measurement is shown as absent, never as 0 — tokens_per_success with zero
  //     passes is the designed case).
  {
    const readme2 = await readFile(join(REPO, "README.md"), "utf8");
    const REQUIRED_SPIKES = [
      "tokens_per_task_mean",
      "tokens_per_success",
      "tokens_measured_tasks",
      "task_latency_ms_p50",
      "task_latency_ms_p95",
      "store_memory_latency_ms_p50",
      "store_memory_latency_ms_p95",
      "recall_memory_latency_ms_p50",
      "recall_memory_latency_ms_p95",
      "query_dataset_latency_ms_p50",
      "query_dataset_latency_ms_p95",
      "rpc_probe_samples",
    ];
    const spikeRows = new Map(
      [...readme2.matchAll(/^\|\s*`([a-z_0-9]+)`\s*\|([^|]*)\|([^|]*)\|/gm)].map((r) => [
        r[1],
        [r[2].trim(), r[3].trim()],
      ]),
    );
    for (const id of REQUIRED_SPIKES) {
      const row = spikeRows.get(id);
      if (!row) {
        problems.push(
          `README.md: no performance-table row for spike '${id}' — a recorded absolute ` +
            `omitted from the table is a number chosen not to show`,
        );
        continue;
      }
      loaded.forEach(([engine, , , spikes], i) => {
        const cell = row[i];
        const spike = spikes.get(id);
        if (!spike) {
          if (cell !== "—") {
            problems.push(
              `README.md: spike '${id}' on ${engine} reads '${cell}' but the baseline ` +
                `recorded no such measurement — an absent measurement is published as '—'`,
            );
          }
          return;
        }
        const expected = `${Math.round(spike.value)} ${spike.unit}`;
        if (cell !== expected) {
          problems.push(
            `README.md: spike '${id}' on ${engine} reads '${cell}', baseline says '${expected}'`,
          );
        }
      });
    }
  }

  // (8) THE NARRATIVE — every metric-shaped number in the prose BELOW the tables must be
  //     traceable to a committed baseline. The tables and the charts have been checked
  //     since they were introduced; the paragraphs explaining them never were, and that is
  //     not a smaller hole than the one check (6) closes — it is the same hole one layer
  //     out. A reader does not compare a sentence to a table sixty lines up; they believe
  //     the sentence. A capture that moves every number leaves the prose describing the
  //     PREVIOUS capture, and nothing here noticed until a whole section was asserting the
  //     opposite of the table above it.
  //
  //     The rule is deliberately mechanical: any 3-or-4 digit integer in the narrative
  //     must appear as a gate per-mille or as a committed spike (raw, or rounded to the
  //     second/minute the prose is entitled to use). Anything genuinely not a measurement
  //     — a year, a byte ceiling, a model size — goes in NON_METRIC with a reason, so the
  //     exemption is a decision on the record rather than a gap in a regex.
  {
    const readme3 = await readFile(join(REPO, "README.md"), "utf8");
    // Anchored on the phrase, not the count — the count is part of the prose and changes
    // with the capture, and a marker that moves with the content is not a marker.
    const START = "are worth explaining";
    const END = "### What this does not measure";
    const from = readme3.indexOf(START);
    const to = readme3.indexOf(END);
    if (from < 0 || to < 0 || to < from) {
      problems.push(
        `README.md: cannot locate the benchmark narrative (looked for '${START}' … '${END}'). ` +
          `This gate reads that region; if the section was renamed, move the markers with it ` +
          `rather than leaving the prose unchecked.`,
      );
    } else {
      const prose = readme3.slice(from, to);

      // Every number a baseline entitles the prose to print.
      const allowed = new Set();
      for (const [, gates, , spikes] of loaded) {
        for (const pm of gates.values()) allowed.add(String(pm));
        for (const s of spikes.values()) {
          const v = Number(s.value);
          allowed.add(String(Math.round(v)));
          if (/ms/i.test(s.unit ?? "")) {
            allowed.add(String(Math.round(v / 1000))); // seconds
            allowed.add(String(Math.round(v / 60000))); // minutes
            allowed.add((v / 1000).toFixed(1));
            allowed.add((v / 60000).toFixed(1));
          }
        }
      }

      // Numbers in this region that are not measurements. Each needs a reason.
      const NON_METRIC = new Map([
        ["2026", "a calendar year"],
        ["1000", "the per-mille ceiling itself, used as a word"],
      ]);

      const seen = new Map();
      for (const m of prose.matchAll(/(?<![\w.\-/])(\d{3,4})(?:\.\d)?(?![\w%])/g)) {
        const n = m[1];
        if (allowed.has(n) || NON_METRIC.has(n)) continue;
        // Report each distinct orphan once, with the sentence it sits in.
        if (seen.has(n)) continue;
        const at = m.index ?? 0;
        const line = prose.slice(prose.lastIndexOf("\n", at) + 1, prose.indexOf("\n", at));
        seen.set(n, line.trim());
      }
      for (const [n, line] of seen) {
        problems.push(
          `README.md: the benchmark narrative prints '${n}', which is not a gate value or a ` +
            `committed spike in either baseline — so it is either stale from an earlier ` +
            `capture or fabricated. Sentence: "${line.slice(0, 120)}${line.length > 120 ? "…" : ""}"`,
        );
      }

      // The set check above only asks whether a number exists SOMEWHERE. That is not
      // enough: a value carried over from a previous capture is very often still a real
      // number for some other gate, so it passes membership while saying something false.
      // Attribution is the check that catches it — when a sentence names a metric and
      // prints a figure, the figure has to be THAT metric's.
      const byGate = new Map();
      for (const [, gates] of loaded) {
        for (const [id, pm] of gates) {
          const base = id.split("@")[0];
          if (!byGate.has(base)) byGate.set(base, new Set());
          byGate.get(base).add(String(pm));
        }
      }
      for (const [, , , spikes] of loaded) {
        for (const [id, s] of spikes) {
          const base = id.split("@")[0];
          if (!byGate.has(base)) byGate.set(base, new Set());
          const v = Number(s.value);
          const set = byGate.get(base);
          set.add(String(Math.round(v)));
          if (/ms/i.test(s.unit ?? "")) {
            set.add(String(Math.round(v / 1000)));
            set.add(String(Math.round(v / 60000)));
            set.add((v / 1000).toFixed(1));
            set.add((v / 60000).toFixed(1));
          }
        }
      }
      for (const sentence of prose.split(/(?<=[.!?])\s+/)) {
        const named = [...sentence.matchAll(/`([a-z][a-z_0-9]*)`/g)]
          .map((m) => m[1])
          .filter((g) => byGate.has(g));
        if (named.length !== 1) continue; // ambiguous or metric-free — the set check covers it
        const gate = named[0];
        const values = byGate.get(gate);
        for (const m of sentence.matchAll(/(?<![\w.\-/])(\d{3,4})(?:\.\d)?(?![\w%])/g)) {
          const n = m[1];
          if (values.has(n) || NON_METRIC.has(n)) continue;
          problems.push(
            `README.md: the narrative attributes '${n}' to \`${gate}\`, but no engine or ` +
              `family records that value for it (committed: ` +
              `${[...values].slice(0, 6).join(", ")}${values.size > 6 ? ", …" : ""}). ` +
              `Sentence: "${sentence.replace(/\s+/g, " ").slice(0, 120)}…"`,
          );
        }
      }
    }
  }
}

for (const source of SOURCES) {
  if (!existsSync(source)) continue;
  const text = await readFile(source, "utf8");
  for (const [, relPath, anchor] of text.matchAll(BLOB)) {
    const target = join(REPO, relPath);
    if (!existsSync(target)) {
      problems.push(`${source}: links to ${relPath}, which does not exist in the repo`);
      continue;
    }
    if (!anchorCache.has(target)) anchorCache.set(target, await anchorsOf(target));
    const anchors = anchorCache.get(target);
    if (!anchors.has(anchor)) {
      problems.push(
        `${source}: ${relPath}#${anchor} — no such heading. ` +
          `Closest: ${[...anchors].filter((a) => a.startsWith(anchor.slice(0, 4))).join(", ") || "(none)"}`,
      );
    }
  }
}

if (problems.length > 0) {
  console.error(`✗ ${problems.length} docs problem(s):\n`);
  for (const p of problems) console.error(`  ${p}`);
  console.error("\nList the page in the sidebar · fix the link or add the heading · move the");
  console.error("README table and the baseline together. Each of these fails silently: an");
  console.error("unreachable page, a link that lands at the top of a long file, and a score");
  console.error("the ratchet no longer holds all look fine until someone relies on them.");
  process.exit(1);
}

console.log(
  `✓ every docs page is in the sidebar, every GitHub-hosted anchor resolves, and the ` +
    `README + evaluation.md benchmark tables, fractions and charts match the committed ` +
    `baselines (${SOURCES.length} files scanned)`,
);
