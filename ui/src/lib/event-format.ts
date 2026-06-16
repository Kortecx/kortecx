/**
 * Pure display helpers for one event `Delta` (the activity feed's rows). Maps the
 * stable lowercase delta `kind` to a reused state tone + a one-line summary. An
 * unrecognized kind renders as `unknown` rather than crashing — mirrors the
 * `stateVisual` philosophy (the proto is additive-only, new kinds are safe).
 */

import type { StateTone } from "./colors";
import { shortHex } from "./format";

/** The minimal shape of an SDK `Delta` the feed needs (structurally compatible
 *  with both the per-run `Delta` and the Batch C `GlobalDelta`). */
export interface EventLike {
  readonly seq: number;
  readonly kind: string;
  readonly instanceId?: string | null;
  readonly moteId?: string | null;
  readonly resultRef?: string | null;
  readonly targetMoteId?: string | null;
  readonly reasonClass?: number | null;
  readonly recipeFingerprint?: string | null;
  readonly ndClass?: number | null;
  readonly targetCommittedSeq?: number | null;
  readonly registeredUnixMs?: number | null;
}

export interface EventVisual {
  readonly label: string;
  /** Reuses the Mote-state tone palette so `.pill--<tone>` styling is shared. */
  readonly tone: StateTone;
}

const KIND_VISUAL: Readonly<Record<string, EventVisual>> = {
  committed: { label: "COMMITTED", tone: "committed" },
  failed: { label: "FAILED", tone: "failed" },
  repudiated: { label: "REPUDIATED", tone: "repudiated" },
  effect_staged: { label: "EFFECT STAGED", tone: "scheduled" },
  // Batch C: the global tail surfaces run starts (the per-run cursor never does).
  run_registered: { label: "RUN STARTED", tone: "scheduled" },
};
const UNKNOWN_VISUAL: EventVisual = { label: "EVENT", tone: "unknown" };

export function eventVisual(kind: string): EventVisual {
  return KIND_VISUAL[kind] ?? UNKNOWN_VISUAL;
}

/**
 * The journal's `FailureReason` discriminant → a short triage label. Mirrors the
 * closed enum in `kx-journal/src/entry.rs` (variants 0-8, in declaration order).
 * An unknown discriminant maps to "UNKNOWN REASON" rather than crashing (the proto
 * is additive-only); a `null`/absent reason returns `null` so a failed row that
 * carried no reason shows NO fabricated label (GR15 — never invent a cause).
 */
const FAILURE_REASON: Readonly<Record<number, string>> = {
  0: "TIMED OUT",
  1: "EXECUTOR REFUSED",
  2: "VALIDATOR REJECTED",
  3: "WORKER CRASHED",
  4: "UPSTREAM REPUDIATED",
  5: "UNSAFE WORLD-MUTATING",
  6: "COMPENSATED",
  7: "QUARANTINED",
  8: "DEAD-LETTERED",
};

export function failureReasonLabel(code: number | null | undefined): string | null {
  if (code === null || code === undefined) {
    return null;
  }
  return FAILURE_REASON[code] ?? "UNKNOWN REASON";
}

/** A one-line human summary of a delta (the Mote it concerns + its effect).
 *  `recipeName` labels a `run_registered` row when the fingerprint→handle join
 *  resolved one (else the row degrades to the fingerprint hex). `omitResultRef`
 *  drops the trailing `→ <hash>` on a committed row when the caller renders the
 *  RESOLVED result text alongside (so the hash never doubles as the headline).
 *  `omitReason` drops the trailing `— <REASON>` on a failed row when the caller
 *  renders the reason as a separate badge (so it never shows twice). */
export function eventSummary(
  d: EventLike,
  recipeName?: string,
  omitResultRef = false,
  omitReason = false,
): string {
  switch (d.kind) {
    case "committed":
      return `Mote ${shortHex(d.moteId ?? "")} committed${
        d.resultRef && !omitResultRef ? ` → ${shortHex(d.resultRef)}` : ""
      }`;
    case "failed": {
      const reason = omitReason ? null : failureReasonLabel(d.reasonClass);
      return `Mote ${shortHex(d.moteId ?? "")} failed${reason ? ` — ${reason}` : ""}`;
    }
    case "repudiated":
      return `Mote ${shortHex(d.targetMoteId ?? "")} repudiated`;
    case "effect_staged":
      return `Mote ${shortHex(d.moteId ?? "")} staged an effect`;
    case "run_registered":
      return recipeName
        ? `Run started — ${recipeName}`
        : `Run started${d.recipeFingerprint ? ` — ${shortHex(d.recipeFingerprint)}` : ""}`;
    default:
      return `event ${d.kind}`;
  }
}

/** The five global-tail event kinds, in the chip-toolbar display order. The
 *  filter is purely CLIENT-SIDE over the buffered feed (W1a-3); the proto is
 *  additive-only, so an unrecognized kind falls through to `unknown`. */
export const FEED_KINDS = [
  "committed",
  "failed",
  "repudiated",
  "effect_staged",
  "run_registered",
] as const;

/** Count the buffered deltas by kind (for the per-chip count badges). Counts are
 *  over the CURRENT ring buffer only — the caller labels them as such (honest;
 *  the buffer is bounded, never "all-time"). An unrecognized kind buckets to
 *  `unknown`. */
export function tallyEventsByKind(events: readonly EventLike[]): Record<string, number> {
  const tally: Record<string, number> = {};
  for (const e of events) {
    const key = (FEED_KINDS as readonly string[]).includes(e.kind) ? e.kind : "unknown";
    tally[key] = (tally[key] ?? 0) + 1;
  }
  return tally;
}

/** The client-side feed filter: a set of enabled kinds (`null` = every kind)
 *  plus a case-insensitive free-text query matched against the row's ids + its
 *  human summary (so a mote-hex prefix OR "TIMED OUT" both narrow). */
export interface FeedFilter {
  readonly kinds: ReadonlySet<string> | null;
  readonly query: string;
}

/** Does a delta pass the filter? An absent kind set shows every kind; a present
 *  set shows only enabled kinds. The query (trimmed, lowercased) substring-matches
 *  the instance/mote/target hex AND the `eventSummary` text. */
export function matchesFeedFilter(d: EventLike, filter: FeedFilter, recipeName?: string): boolean {
  if (filter.kinds && !filter.kinds.has(d.kind)) {
    return false;
  }
  const q = filter.query.trim().toLowerCase();
  if (q === "") {
    return true;
  }
  const haystack = [
    d.instanceId ?? "",
    d.moteId ?? "",
    d.targetMoteId ?? "",
    eventSummary(d, recipeName),
  ]
    .join(" ")
    .toLowerCase();
  return haystack.includes(q);
}

/** Serialize the (filtered) feed to NDJSON — one server-derived object per line,
 *  matching the CLI `kx events --all --json` shape per kind (snake_case `type` +
 *  hex join keys ONLY; never payloads/secrets — SN-8). The result text is NOT
 *  exported here (it is content-addressed; the hash is the join key). */
export function feedToNdjson(deltas: readonly EventLike[]): string {
  return deltas.map((d) => JSON.stringify(deltaToWire(d))).join("\n");
}

/** Map an `NdClass` discriminant to its lowercase wire tag — mirrors the runtime
 *  `nd_class_tag` / the WS `nd_str` so the export's `nd_class` is byte-identical
 *  to `kx events --all --json` (a STRING, never a number). Unknown ⇒ "unspecified". */
function ndClassTag(nd: number | null | undefined): string {
  switch (nd) {
    case 1:
      return "pure";
    case 2:
      return "read_only_nondet";
    case 3:
      return "world_mutating";
    default:
      return "unspecified";
  }
}

function deltaToWire(d: EventLike): Record<string, unknown> {
  const base = { seq: d.seq, instance_id: d.instanceId ?? "" };
  switch (d.kind) {
    case "committed":
      return {
        ...base,
        type: "committed",
        mote_id: d.moteId ?? "",
        result_ref: d.resultRef ?? "",
        nd_class: ndClassTag(d.ndClass),
      };
    case "failed":
      return { ...base, type: "failed", mote_id: d.moteId ?? "", reason_class: d.reasonClass ?? 0 };
    case "repudiated":
      return {
        ...base,
        type: "repudiated",
        target_mote_id: d.targetMoteId ?? "",
        target_committed_seq: d.targetCommittedSeq ?? 0,
      };
    case "effect_staged":
      return { ...base, type: "effect_staged", mote_id: d.moteId ?? "" };
    case "run_registered":
      return {
        ...base,
        type: "run_registered",
        recipe_fingerprint: d.recipeFingerprint ?? "",
        registered_unix_ms: d.registeredUnixMs ?? 0,
      };
    default:
      return { ...base, type: "unknown" };
  }
}

/** A safe, slugged filename for an exported feed (the `exportRunFilename`
 *  precedent — never empty, no path chars). */
export function exportFeedFilename(now: number = Date.now()): string {
  return `kortecx-feed-${now}.ndjson`;
}
