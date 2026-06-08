/** Builders for `Projection` / `MoteView` (the real SDK classes) used by tests. */

import { MoteView, ParentEdge, Projection } from "@kortecx/sdk/web";

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
