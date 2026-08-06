/** Builders for `Projection` / `MoteView` (the real SDK classes) used by tests. */

import { MoteView, ParentEdge, Projection, ReactTurn } from "@kortecx/sdk/web";

let counter = 0;

/** A deterministic 32-byte hex id from a small integer (stable across a test run). */
export function nid(n: number): string {
  return n.toString(16).padStart(64, "0");
}

export interface ParentOpt {
  parentId: string;
  edgeKind?: "data" | "control" | "unknown";
  nonCascade?: boolean;
}

export interface MoteOpts {
  moteId?: string;
  stateCode?: number;
  ndClass?: number;
  promotion?: number;
  resultRef?: string | null;
  moteDefHash?: string;
  committedSeq?: number | null;
  anomaly?: number | null;
  parents?: ParentOpt[];
}

export function mote(opts: MoteOpts = {}): MoteView {
  const id = opts.moteId ?? nid(counter++);
  const parents = (opts.parents ?? []).map(
    (p) => new ParentEdge(p.parentId, p.edgeKind ?? "data", p.nonCascade ?? false),
  );
  return new MoteView(
    id,
    "STATE", // display name — unused by the VM (it reads stateCode)
    opts.stateCode ?? 3,
    opts.ndClass ?? 1,
    opts.promotion ?? 1,
    opts.resultRef ?? null,
    opts.moteDefHash ?? "cd".repeat(32),
    opts.committedSeq ?? null,
    opts.anomaly ?? null,
    parents,
  );
}

export interface ProjectionOpts {
  instanceId?: string;
  recipeFingerprint?: string;
  currentSeq?: number;
}

export function projection(motes: MoteView[], opts: ProjectionOpts = {}): Projection {
  return new Projection(
    opts.instanceId ?? "ab".repeat(16),
    opts.recipeFingerprint ?? "ef".repeat(32),
    opts.currentSeq ?? motes.length,
    motes,
  );
}

/** One Mote in each state code 0..6 (covers all states + UNSPECIFIED). */
export function allStatesProjection(): Projection {
  return projection([0, 1, 2, 3, 4, 5, 6].map((s) => mote({ stateCode: s })));
}

/** A large projection for the render perf budget. */
export function largeProjection(n: number): Projection {
  const motes = Array.from({ length: n }, (_, i) =>
    mote({ moteId: nid(i), stateCode: (i % 6) + 1 }),
  );
  return projection(motes, { currentSeq: n });
}

// ---- Multi-node DAG topologies (T3.3) ---------------------------------------

/** A linear chain a → b → c → … of `n` Motes (deep-chain layout). */
export function chainProjection(n: number): Projection {
  const motes = Array.from({ length: n }, (_, i) =>
    mote({ moteId: nid(i), parents: i === 0 ? [] : [{ parentId: nid(i - 1) }] }),
  );
  return projection(motes);
}

/** A diamond a → {b, c} → d (the classic relayout / fan-out-then-join shape). */
export function diamondProjection(): Projection {
  const a = mote({ moteId: nid(0) });
  const b = mote({ moteId: nid(1), parents: [{ parentId: nid(0) }] });
  const c = mote({ moteId: nid(2), parents: [{ parentId: nid(0) }] });
  const d = mote({ moteId: nid(3), parents: [{ parentId: nid(1) }, { parentId: nid(2) }] });
  return projection([a, b, c, d]);
}

/** One root fanning out to `n` leaves. */
export function fanOutProjection(n: number): Projection {
  const root = mote({ moteId: nid(0) });
  const leaves = Array.from({ length: n }, (_, i) =>
    mote({ moteId: nid(i + 1), parents: [{ parentId: nid(0) }] }),
  );
  return projection([root, ...leaves]);
}

/** `n` roots converging on one gather Mote. */
export function fanInProjection(n: number): Projection {
  const roots = Array.from({ length: n }, (_, i) => mote({ moteId: nid(i) }));
  const gather = mote({
    moteId: nid(n),
    parents: roots.map((_, i) => ({ parentId: nid(i) })),
  });
  return projection([...roots, gather]);
}

/** Two independent subgraphs (multi-root layout): a→b and c→d. */
export function disconnectedProjection(): Projection {
  return projection([
    mote({ moteId: nid(0) }),
    mote({ moteId: nid(1), parents: [{ parentId: nid(0) }] }),
    mote({ moteId: nid(2) }),
    mote({ moteId: nid(3), parents: [{ parentId: nid(2) }] }),
  ]);
}

/** A child with one DATA, one CONTROL, and one non-cascade CONTROL parent. */
export function controlEdgeProjection(): Projection {
  return projection([
    mote({ moteId: nid(0) }),
    mote({ moteId: nid(1) }),
    mote({ moteId: nid(2) }),
    mote({
      moteId: nid(3),
      parents: [
        { parentId: nid(0), edgeKind: "data" },
        { parentId: nid(1), edgeKind: "control" },
        { parentId: nid(2), edgeKind: "control", nonCascade: true },
      ],
    }),
  ]);
}

/**
 * A run that GROWS between polls (the PR-2b dynamic-shaper-child beat):
 *  - frame 0: root SCHEDULED, alone;
 *  - frame 1: root COMMITTED + two PENDING children appear (topology grows);
 *  - frame 2: children COMMITTED (state-only change — no new topology).
 */
export function growsBetweenPolls(): [Projection, Projection, Projection] {
  const root = (state: number) => mote({ moteId: nid(0), stateCode: state });
  const child = (i: number, state: number) =>
    mote({ moteId: nid(i), stateCode: state, parents: [{ parentId: nid(0) }] });
  return [
    projection([root(2)], { currentSeq: 1 }),
    projection([root(3), child(1, 1), child(2, 1)], { currentSeq: 3 }),
    projection([root(3), child(1, 3), child(2, 3)], { currentSeq: 5 }),
  ];
}

/** A defensive malformed input: a 2-cycle a↔b. The DAG must render (no hang). */
export function cycleProjection(): Projection {
  return projection([
    mote({ moteId: nid(0), parents: [{ parentId: nid(1) }] }),
    mote({ moteId: nid(1), parents: [{ parentId: nid(0) }] }),
  ]);
}

// ---- ReAct / agentic topologies (W2) ----------------------------------------
//
// WHY THESE EXIST. Every multi-node fixture above is ONE connected component, so
// nothing in the suite has ever fed `scopeProjection`/`connectedComponent` the shape
// a real agent run folds to. The coordinator registers a react TURN Mote EDGE-FREE
// (`react_shape.rs` — `SmallVec::new()`, frozen by "a react turn MUST be edge-free")
// because declaring parents would move the canonical digest, and a tool OBSERVATION
// carries exactly ONE Data edge back to the turn that proposed it. So an N-turn run
// is N DISJOINT TWO-NODE STARS, and an undirected walk from any anchor reaches one
// star. These builders reproduce that exactly.

/** The react chain SALT — the seed Mote the coordinator validates and then DISCARDS.
 *  It is deliberately absent from every projection below: that is the whole point. */
export const REACT_SEED = nid(0x5eed);
/** The ADMITTED agentic launch Mote (the `kx chat --tools` / SubmitWorkflow shape).
 *  The loop's answer commits onto THIS Mote, which is why the answer turn must not
 *  also be rendered — the same bytes would appear twice. */
export const REACT_LAUNCH = nid(0x300);
/** The run-salted turn Mote for turn `k`. */
export const turnId = (k: number): string => nid(0x100 + k);
/** The observation Mote for turn `k`'s tool call (single Data edge → its turn). */
export const obsId = (k: number): string => nid(0x200 + k);
/** The content ref the answer's bytes live at. */
export const ANSWER_REF = "aa".repeat(32);

export interface ReactChainOpts {
  /** How many turns the chain HAS (turn 0 .. turns-1; the last one answers). */
  turns: number;
  /** Mid-poll frontier: how many turn Motes have actually LANDED in the fold.
   *  Defaults to all of them (a settled run). */
  presentTurns?: number;
  /** Add the admitted launch Mote carrying the answer bytes (the `chat --tools` shape). */
  launch?: boolean;
  /** Turn indices whose Motes are absent from the fold (a hole in the middle). */
  absentTurns?: readonly number[];
}

/**
 * A REACT-SHAPED fold: `turns` edge-free turn Motes, each non-answering turn carrying
 * ONE observation whose single Data parent is that turn. No edge joins turn k to
 * turn k+1 — the runtime records none.
 */
export function reactChainProjection(opts: ReactChainOpts): Projection {
  const total = opts.turns;
  const present = opts.presentTurns ?? total;
  const absent = new Set(opts.absentTurns ?? []);
  const motes: MoteView[] = [];
  if (opts.launch) {
    // The launch step is admitted and the loop's answer commits onto it.
    motes.push(mote({ moteId: REACT_LAUNCH, stateCode: 3, resultRef: ANSWER_REF }));
  }
  for (let k = 0; k < total; k += 1) {
    if (k >= present || absent.has(k)) {
      continue; // not at this frontier yet
    }
    const answers = k === total - 1;
    motes.push(
      mote({
        moteId: turnId(k),
        stateCode: 3,
        parents: [], // EDGE-FREE — the defect's root cause, reproduced faithfully
        resultRef: answers && !opts.launch ? ANSWER_REF : null,
      }),
    );
    if (!answers) {
      motes.push(mote({ moteId: obsId(k), stateCode: 3, parents: [{ parentId: turnId(k) }] }));
    }
  }
  return projection(motes, { currentSeq: motes.length });
}

export interface ReactRowsOpts {
  turns: number;
  /** The chain key. ⚠ Pass `""` for the UNSALTED run-level shape a real
   *  `kx agent run` actually produces — measured live, its rows carry no key at all. */
  stepSalt?: string;
  /** Fan this turn into `multiToolCount` `tool` rows sharing one `turnMoteId`. */
  multiToolAt?: number;
  multiToolCount?: number;
  /** Emit the last turn as still `pending` rather than `answer` (a live chain). */
  unsettled?: boolean;
}

/**
 * The matching `ListReactTurns` page — NEWEST-FIRST (descending seq), exactly as the
 * wire delivers it, so a reader's dedupe/order rule is exercised for real.
 */
export function reactTurnRows(opts: ReactRowsOpts): { turns: ReactTurn[]; hasMore: boolean } {
  const salt = opts.stepSalt ?? REACT_SEED;
  let seq = 10;
  const rows: ReactTurn[] = [];
  for (let k = 0; k < opts.turns; k += 1) {
    const answers = k === opts.turns - 1;
    const branch = answers ? (opts.unsettled ? "pending" : "answer") : "tool";
    const fan = k === opts.multiToolAt ? (opts.multiToolCount ?? 2) : 1;
    for (let ci = 0; ci < fan; ci += 1) {
      rows.push(
        new ReactTurn(
          k,
          turnId(k),
          "ab".repeat(16),
          "gemma4:12b",
          branch,
          branch === "tool" ? "mcp-echo" : "",
          branch === "tool" ? "1" : "",
          8,
          8,
          seq++,
          "",
          salt,
          ci,
          ["mcp-echo@1"],
          [],
        ),
      );
    }
  }
  rows.reverse(); // newest-first, like the wire
  return { turns: rows, hasMore: false };
}
