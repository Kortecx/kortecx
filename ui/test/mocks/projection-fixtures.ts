/** Builders for `Projection` / `MoteView` (the real SDK classes) used by tests. */

import { MoteView, Projection } from "@kortecx/sdk/web";

let counter = 0;

export interface MoteOpts {
  moteId?: string;
  stateCode?: number;
  ndClass?: number;
  promotion?: number;
  resultRef?: string | null;
  moteDefHash?: string;
  committedSeq?: number | null;
  anomaly?: number | null;
}

export function mote(opts: MoteOpts = {}): MoteView {
  const id = opts.moteId ?? (counter++).toString(16).padStart(64, "0");
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
    mote({ moteId: i.toString(16).padStart(64, "0"), stateCode: (i % 6) + 1 }),
  );
  return projection(motes, { currentSeq: n });
}
