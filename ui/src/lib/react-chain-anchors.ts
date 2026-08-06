/**
 * Recover a run's chain — and its scoping anchors — from the durable turn record.
 *
 * A run launched outside this browser (`kx agent run`, `kx chat --tools`, a trigger)
 * appears in the durable run list, but that enumeration carries no anchor: it lists
 * INSTANCES, and one `kx serve` is one journal with one instance id shared by every
 * submission. Nothing server-side stores a submission's anchor either.
 *
 * The chains ARE recoverable, because they are durable facts. But there are two shapes,
 * and only one of them is keyed:
 *
 *  - **Salted.** A tool-granted agentic step carries a chain key, and grouping by that
 *    key separates submissions exactly.
 *  - **Unsalted.** ⚠ A real `kx agent run` produces rows with NO chain key at all — the
 *    run-level shape. Measured live: every row of a 3-turn chain carried an absent salt.
 *    Grouping those by their (empty) key would MERGE every such run on the node into one
 *    fictional chain, so they are segmented by their own turn numbering instead: turns
 *    ascend within a chain and restart at 0 for the next, so a non-increasing turn on
 *    seq-ordered rows is a chain boundary.
 *
 * That second shape is not an edge case — it is the shape this whole surface exists for,
 * and treating it as unscopable (the first version of this module did) disables the
 * feature for precisely the runs it was built to reach.
 *
 * Pure and fail-closed: a chain that cannot be identified yields nothing rather than a
 * guess, because a fabricated anchor presents somebody else's run as this one.
 */

/** The `ListReactTurns` fields this needs (SDK-free, so the module stays a pure lib). */
export interface ChainRowLike {
  readonly turn: number;
  readonly turnMoteId: string;
  /** The chain key; EMPTY/absent for the unsalted run-level shape. */
  readonly stepSalt?: string | null;
  /** Journal sequence — the only true ordering key across chains. */
  readonly seq?: number;
}

/** One agentic submission, as the reader reconstructs it. */
export interface ChainAnchors {
  /** The chain key when the shape carries one; EMPTY for an unsalted chain. */
  readonly stepSalt: string;
  /** The lowest-numbered turn's Mote — the admitted anchor, and the one that resolves. */
  readonly turn0MoteId: string;
  /** Every turn Mote in the chain, in turn order. The membership test AND the roster. */
  readonly turnMoteIds: readonly string[];
  /** How many distinct turns the chain holds (display only, never an assertion). */
  readonly turns: number;
}

/**
 * Ascending by `seq` — the only true cross-chain ordering, since a turn number restarts
 * per chain and a turn carries no clock at all.
 *
 * Rows without a seq are left in the order given. Segmenting unsalted chains needs
 * journal order, and sorting those by `turn` would INTERLEAVE two chains into one
 * ascending run and hide the very boundary the segmentation looks for. Every real row
 * carries a seq; this branch exists so a caller that hand-builds rows gets its own order
 * respected rather than silently scrambled.
 */
function inJournalOrder(rows: readonly ChainRowLike[]): ChainRowLike[] {
  return rows.every((r) => typeof r.seq === "number")
    ? [...rows].sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0))
    : [...rows];
}

function toAnchors(stepSalt: string, rows: readonly ChainRowLike[]): ChainAnchors | null {
  const byTurn = new Map<number, string>();
  for (const r of rows) {
    if (!byTurn.has(r.turn)) {
      byTurn.set(r.turn, r.turnMoteId);
    }
  }
  const turns = [...byTurn.entries()].sort((a, b) => a[0] - b[0]);
  const first = turns[0];
  if (first === undefined) {
    return null;
  }
  return {
    stepSalt,
    turn0MoteId: first[1],
    turnMoteIds: turns.map(([, id]) => id),
    turns: turns.length,
  };
}

/**
 * Reconstruct every agentic chain in a page of turn rows.
 *
 * Salted rows group by their key. Unsalted rows are segmented by turn restart, in
 * journal order — the only signal that separates two run-level chains under one
 * instance, because they share an instance id, carry no key, and have no clock.
 */
export function chainAnchors(rows: readonly ChainRowLike[]): ChainAnchors[] {
  const salted = new Map<string, ChainRowLike[]>();
  const unsalted: ChainRowLike[] = [];
  for (const r of rows) {
    const salt = r.stepSalt ?? "";
    if (salt) {
      const bucket = salted.get(salt);
      if (bucket === undefined) {
        salted.set(salt, [r]);
      } else {
        bucket.push(r);
      }
    } else {
      unsalted.push(r);
    }
  }
  const out: ChainAnchors[] = [];
  for (const [salt, group] of salted) {
    const a = toAnchors(salt, group);
    if (a) {
      out.push(a);
    }
  }
  let segment: ChainRowLike[] = [];
  let prevTurn = Number.NEGATIVE_INFINITY;
  const flush = () => {
    const a = segment.length > 0 ? toAnchors("", segment) : null;
    if (a) {
      out.push(a);
    }
    segment = [];
  };
  for (const r of inJournalOrder(unsalted)) {
    if (r.turn < prevTurn) {
      flush(); // turn went backwards ⇒ a new run-level chain started
    }
    segment.push(r);
    prevTurn = r.turn;
  }
  flush();
  return out;
}

/**
 * The chain that contains `moteId` as one of its turns, or `null`.
 *
 * This is how the run view identifies its own chain, and it is a MEMBERSHIP test rather
 * than a key lookup on purpose: the run view's `?chain=` is the chain key only when the
 * server emitted one, and for a plain `kx agent run` it is the turn-0 Mote instead. A
 * key lookup silently finds nothing there; membership finds the right chain in both
 * shapes and can never select a different run's.
 */
export function chainContaining(
  chains: readonly ChainAnchors[],
  moteId: string,
): ChainAnchors | null {
  if (!moteId) {
    return null;
  }
  for (const c of chains) {
    if (c.stepSalt === moteId || c.turnMoteIds.includes(moteId)) {
      return c;
    }
  }
  return null;
}
