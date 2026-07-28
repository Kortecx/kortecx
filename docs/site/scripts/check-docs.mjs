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
    loaded.push([name, new Map(doc.gates.map((g) => [g.id, g.per_mille])), doc.env]);
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
  /** @type {Map<string, number>} suite-wide binary metric → its honest denominator */
  const binaryDenominators = new Map([
    ["task_success", suiteTotal],
    ["injection_resistance", injectionTaskCount],
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
      if (!claimed.has(family)) {
        problems.push(
          `${source}: the benchmark table has no row for the '${family}' family, which the ` +
            `corpus contains — an unpublished family is a measured capability the reader never sees`,
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
      loaded.forEach((_, i) => {
        const parsed = CELL.exec(suiteSuccessRow[3 + i].trim());
        if (parsed && readmeTable.sums[i] !== Number(parsed[2])) {
          problems.push(
            `README.md: the family pass-counts sum to ${readmeTable.sums[i]} on ` +
              `${loaded[i][0]}, but suite-wide task_success claims ${parsed[2]}/${suiteTotal}`,
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

  // (6) THE README's DENOMINATOR CHARTS. A bar chart of family rates hides that a
  //     1000 from one task is one pass while a 1000 from six is six — so each chart
  //     encodes the per-engine fraction IN its x-axis labels, and this check holds the
  //     labels and the bars to the same baselines and corpus counts as the tables.
  //     The `<!-- bench-chart:<key> -->` anchor comment is the contract: deleting it
  //     (or the chart) fails here, by design.
  {
    const chartKeys = [
      ["Ollama", "ollama"],
      ["llama.cpp", "llamacpp"],
    ];
    chartKeys.forEach(([engine, key], engineIdx) => {
      const anchored = new RegExp(
        String.raw`<!--\s*bench-chart:${key}\b[^>]*-->\s*` + "```mermaid\\n([\\s\\S]*?)```",
      ).exec(readme);
      if (!anchored) {
        problems.push(
          `README.md: no <!-- bench-chart:${key} --> anchored mermaid chart — the denominator ` +
            `chart is part of the published record; keep the anchor comment with the fence`,
        );
        return;
      }
      const body = anchored[1];
      const axis = /^\s*x-axis\s*\[([^\]]*)\]/m.exec(body);
      const bars = /^\s*bar\s*\[([^\]]*)\]/m.exec(body);
      if (!axis || !bars) {
        problems.push(`README.md: bench-chart:${key} has no x-axis or bar line`);
        return;
      }
      const labels = [...axis[1].matchAll(/"([a-z]+)\s*\((\d+)\/(\d+)\)"/g)];
      const values = bars[1].split(",").map((v) => Number(v.trim()));
      if (labels.length !== taskCounts.size || values.length !== taskCounts.size) {
        problems.push(
          `README.md: bench-chart:${key} plots ${labels.length} label(s) / ${values.length} ` +
            `bar(s); the corpus has ${taskCounts.size} families`,
        );
        return;
      }
      const gate = loaded[engineIdx]?.[1];
      labels.forEach(([, family, num, den], i) => {
        if (readmeTable.order[i] !== undefined && family !== readmeTable.order[i]) {
          problems.push(
            `README.md: bench-chart:${key} order diverges from the family table at position ` +
              `${i} ('${family}' vs '${readmeTable.order[i]}')`,
          );
        }
        const expectedCount = taskCounts.get(family);
        if (expectedCount === undefined) {
          problems.push(`README.md: bench-chart:${key} plots a '${family}' family the corpus does not have`);
          return;
        }
        if (Number(den) !== expectedCount) {
          problems.push(
            `README.md: bench-chart:${key} label '${family} (${num}/${den})' — the corpus has ` +
              `${expectedCount} task(s)`,
          );
        }
        const actual = gate?.get(`task_success@${family}`);
        if (actual !== undefined && values[i] !== actual) {
          problems.push(
            `README.md: bench-chart:${key} bar for '${family}' reads ${values[i]}, ` +
              `baseline.${engine} says ${actual}`,
          );
        }
        if (Math.floor((1000 * Number(num)) / Number(den)) !== values[i]) {
          problems.push(
            `README.md: bench-chart:${key} label '${family} (${num}/${den})' folds to ` +
              `${Math.floor((1000 * Number(num)) / Number(den))} — the bar reads ${values[i]}`,
          );
        }
      });
    });
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
