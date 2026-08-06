/**
 * The run-projection poll — the data layer's centerpiece and the T3.3 forward seam.
 * The run-detail view consumes only this hook; T3.3 swaps the *view* (table → DAG)
 * without touching the data layer. When T3.3 exposes `parents[]` on the SDK's
 * `MoteView`, `toProjectionVM` gains a `parents` field and nothing else changes.
 *
 * We poll `GetProjection` (unary gRPC-web) and stop once the run is at rest. We map
 * the SDK's `Projection` *class* into a PLAIN view-model so TanStack Query's
 * structural sharing keeps a stable reference across unchanged polls (memoized rows
 * then skip re-render).
 */

import type { Projection } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useRef } from "react";
import { componentOfAny } from "../components/dag/dag-graph";
import { isTerminalState } from "../lib/colors";
import { chainAnchors, chainContaining } from "../lib/react-chain-anchors";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

const POLL_MS = 1000;
/** One page of turn rows across every chain on the node — the server clamps this. */
const CHAIN_PAGE = 500;

/** One inbound DAG edge (server-derived hex parent id + edge meta). */
export interface ParentEdgeVM {
  readonly parentId: string;
  readonly edgeKind: "data" | "control" | "unknown";
  readonly nonCascade: boolean;
}

/** A plain, serializable Mote view-model (no class methods → clean structural sharing). */
export interface MoteVM {
  readonly moteId: string;
  readonly stateCode: number;
  readonly ndClass: number;
  readonly promotion: number;
  readonly resultRef: string | null;
  readonly committedSeq: number | null;
  readonly anomaly: number | null;
  /** The committed def hash (hex); EMPTY until the Mote commits — the
   *  inspector's `GetMoteDetail` gate (PR-2). Off the DAG layout hash. */
  readonly moteDefHash: string;
  /** Inbound DAG edges — the source of the live graph's links (empty for a root). */
  readonly parents: readonly ParentEdgeVM[];
}

export interface ProjectionVM {
  readonly instanceId: string;
  readonly recipeFingerprint: string;
  readonly currentSeq: number;
  readonly motes: MoteVM[];
  /**
   * The agentic chain's node roster, in turn order — derived from the run's durable
   * ReactRound facts, NOT from any Mote's `parents[]` (a turn Mote is edge-free by
   * design). Present only for a run with an agentic chain; the DAG turns it into the
   * synthesised edges that draw the turn order. Absent ⇒ nothing to draw.
   */
  readonly agenticTurnIds?: readonly string[];
}

/** Map the SDK's `Projection` (class) to the plain VM the views consume. */
export function toProjectionVM(p: Projection): ProjectionVM {
  return {
    instanceId: p.instanceId,
    recipeFingerprint: p.recipeFingerprint,
    currentSeq: p.currentSeq,
    motes: p.motes.map((m) => ({
      moteId: m.moteId,
      stateCode: m.stateCode,
      ndClass: m.ndClass,
      promotion: m.promotion,
      resultRef: m.resultRef,
      committedSeq: m.committedSeq,
      anomaly: m.anomaly,
      moteDefHash: m.moteDefHash,
      parents: m.parents.map((e) => ({
        parentId: e.parentId,
        edgeKind: e.edgeKind,
        nonCascade: e.nonCascade,
      })),
    })),
  };
}

/** A run is "at rest" when it has Motes and they are all terminal (stop polling). */
export function allTerminal(p: ProjectionVM): boolean {
  return p.motes.length > 0 && p.motes.every((m) => isTerminalState(m.stateCode));
}

/**
 * Whether the run has settled — the cosmetic "live / at rest" signal. When the
 * recipe's terminal (sink) Mote id is known, it reaching a terminal state is
 * authoritative; otherwise fall back to "all visible Motes terminal".
 */
export function runSettled(p: ProjectionVM, terminalMoteId?: string): boolean {
  if (terminalMoteId) {
    return p.motes.some((m) => m.moteId === terminalMoteId && isTerminalState(m.stateCode));
  }
  return allTerminal(p);
}

/**
 * Whether polling can stop. The terminal (sink) Mote committing is authoritative —
 * it commits only AFTER the whole DAG does, so this is correct even while children
 * are still registering (incremental materialization / dynamic shaper children),
 * which a naive "all currently-visible Motes terminal" check gets wrong (it can
 * fire when only the root is present). Without a terminal id (a direct-URL nav),
 * fall back to a frontier-stability heuristic: every visible Mote terminal AND the
 * journal frontier (`current_seq`) did not advance this poll.
 */
export function isRunAtRest(
  data: ProjectionVM,
  terminalMoteId: string | undefined,
  prevSeq: number,
): boolean {
  if (terminalMoteId) {
    return runSettled(data, terminalMoteId);
  }
  return allTerminal(data) && data.currentSeq === prevSeq;
}

export interface UseProjectionOptions {
  atSeq?: number;
  /** The recipe's terminal (sink) Mote id — the authoritative run-complete signal. */
  terminalMoteId?: string;
  /**
   * Scope the returned Motes to the connected component containing this one — i.e. to a
   * SINGLE submission.
   *
   * `GetProjection` is not run-scoped by design: one `kx serve` is one journal with ONE
   * `instance_id` shared by every Invoke, chat turn, scaffold and cron fire, so an
   * unscoped fold returns the whole workspace. Any server-derived Mote id belonging to
   * this submission works as the anchor; `RunHandle.react_chain_salt` is the one the
   * gateway already returns for a run with a single agentic step.
   *
   * When the anchor is absent from the projection the motes are left UNSCOPED and
   * `scopeMissed` is true. (The narrowing is dropped rather than emptied so the run view
   * can still render something; that only stays honest because every consumer checks the
   * flag FIRST — see {@link ScopedProjectionVM.scopeMissed}. A consumer that reads
   * `motes` without it will silently present the whole journal as the run.)
   */
  scopeMoteId?: string;
  /**
   * A second anchor tried only when `scopeMoteId` resolves to nothing (see
   * {@link scopeProjection}). The run view passes its `terminal` search key.
   */
  scopeFallbackMoteId?: string;
}

/** A projection plus whether a requested scope could be applied. */
export interface ScopedProjectionVM extends ProjectionVM {
  /**
   * True when a `scopeMoteId` was requested but is not present in the fold — a stale
   * link, or a journal rebuilt under the same endpoint. `motes` then holds the UNSCOPED
   * fold, so this flag is the only thing standing between the user and every other run's
   * Motes presented as this run's — the exact bug the option exists to fix. CHECK IT
   * BEFORE READING `motes`, and say "could not isolate this run" rather than describing
   * what is in the array.
   */
  readonly scopeMissed: boolean;
}

/**
 * Narrow a projection to one submission (see {@link UseProjectionOptions.scopeMoteId}).
 *
 * `scopeFallbackMoteId` is a SECOND anchor tried only when the first resolves to
 * nothing. The run view passes `terminal` here: a react Invoke's `chain=` carries the
 * chain SALT (the Timeline's `ListReactTurns` key), and from a server that returned
 * the discarded seed as its anchor the salt names no Mote — the terminal (the
 * admitted turn-0) still does. `scopeMissed` is true only when BOTH miss.
 */
export function scopeProjection(
  p: ProjectionVM,
  scopeMoteId?: string,
  scopeFallbackMoteId?: string,
  derivedAnchors: readonly string[] = [],
): ScopedProjectionVM {
  const present = new Set(p.motes.map((m) => m.moteId));
  // The URL anchor resolves EXACTLY as before: primary, else the distinct fallback.
  // (`connectedComponent` is non-empty iff the anchor is in the fold, so a presence
  // check is the same decision, made once.)
  let urlAnchor: string | undefined;
  if (scopeMoteId && present.has(scopeMoteId)) {
    urlAnchor = scopeMoteId;
  } else if (
    scopeMoteId &&
    scopeFallbackMoteId &&
    scopeFallbackMoteId !== scopeMoteId &&
    present.has(scopeFallbackMoteId)
  ) {
    urlAnchor = scopeFallbackMoteId;
  }
  // The chain's turns are ADDITIONAL anchors, never a substitute for the fallback: a
  // react Invoke depends on that fallback and always will, because its `chain=` names
  // the seed the coordinator discards.
  const anchors = [
    ...(urlAnchor ? [urlAnchor] : []),
    ...derivedAnchors.filter((id) => present.has(id)),
  ];
  if (!scopeMoteId && anchors.length === 0) {
    return { ...p, scopeMissed: false }; // never asked
  }
  if (anchors.length === 0) {
    return { ...p, scopeMissed: true }; // asked, and nothing resolved
  }
  return { ...p, motes: [...componentOfAny(p.motes, anchors)], scopeMissed: false };
}

/** One `ListReactTurns` row, narrowed to what the roster needs. */
interface ReactRowLike {
  readonly turn: number;
  readonly callIndex: number;
  readonly branch: string;
  readonly turnMoteId: string;
}

/**
 * The ordered node roster of one agentic chain, from its `ListReactTurns` rows.
 *
 * Deduped by turn (a turn that fires N tools at once fans into N rows sharing one
 * `turnMoteId`) and ordered by `(turn, callIndex)` with FIRST-WINS over the newest-first
 * wire, so a settled row supersedes the same turn's earlier pending one.
 *
 * ⚠ That ordering rule is deliberately RESTATED here rather than imported from
 * `use-react-progress`: this module is in the eager entry chunk and that one is not, so
 * importing it would pull a whole lazy chunk forward into the entry bundle. Both copies
 * are pinned by their own tests.
 *
 * `launchMoteId` is the primary anchor WHEN IT RESOLVED in the fold — which means the
 * run is the agentic-launch shape, and the loop's answer commits onto that launch Mote.
 * The answering turn is then REPLACED by the launch rather than appended, or the same
 * answer bytes would render on two nodes.
 */
export function agenticLineage(
  rows: readonly ReactRowLike[],
  launchMoteId?: string,
): readonly string[] {
  const seen = new Map<string, ReactRowLike>();
  for (const r of rows) {
    const key = `${r.turn}:${r.callIndex}`;
    if (!seen.has(key)) {
      seen.set(key, r);
    }
  }
  const ordered = [...seen.values()].sort((a, b) => a.turn - b.turn || a.callIndex - b.callIndex);
  const roster: string[] = [];
  let absorbed = false;
  for (const r of ordered) {
    const absorbs = launchMoteId !== undefined && r.branch === "answer";
    if (absorbs) {
      absorbed = true;
    }
    const id = absorbs && launchMoteId !== undefined ? launchMoteId : r.turnMoteId;
    if (roster.at(-1) !== id) {
      roster.push(id);
    }
  }
  // A live chain has not answered yet, so the launch is not on the roster — add it so
  // the drawn chain still terminates where the run's output will land.
  if (launchMoteId && !absorbed && roster.at(-1) !== launchMoteId) {
    roster.push(launchMoteId);
  }
  return roster;
}

export function useProjection(instanceId: string | undefined, opts: UseProjectionOptions = {}) {
  const { client, endpoint, status } = useConnection();
  const atSeq = opts.atSeq;
  const terminalMoteId = opts.terminalMoteId;
  const scopeMoteId = opts.scopeMoteId;
  const scopeFallbackMoteId = opts.scopeFallbackMoteId;
  // Tracks the journal frontier across polls for the fallback stop heuristic.
  const frontier = useRef<{ key: string; lastSeq: number }>({ key: "", lastSeq: -1 });
  return useQuery({
    // The scope is part of the identity: two views of the same instance scoped to
    // different submissions are different data, not a cache hit.
    queryKey: [
      ...queryKeys.projection(endpoint, instanceId ?? "", atSeq),
      scopeMoteId ?? "",
      scopeFallbackMoteId ?? "",
    ],
    enabled: status === "connected" && client !== null && Boolean(instanceId),
    queryFn: async (): Promise<ScopedProjectionVM> => {
      if (!client || !instanceId) {
        throw new Error("not connected");
      }
      const view = await client.getProjection(
        instanceId,
        atSeq != null ? { atSeq: BigInt(atSeq) } : {},
      );
      const vm = toProjectionVM(view);
      // An agentic run's chain is durable but OFF-DAG: its turn Motes carry no parents,
      // so an undirected walk from any anchor reaches one two-node star and the rest of
      // the run is invisible. Read the chain's own facts and use each turn as a further
      // anchor. Fetched HERE, inside the query fn, so the growing roster never enters
      // the query key and mints a cache entry per poll.
      //
      // ⚠ Fetched WITHOUT a chain key and selected by MEMBERSHIP. Keying the request on
      // `scopeMoteId` reads as the obvious thing and is wrong for the shape that matters
      // most: measured live, a real `kx agent run` produces rows carrying NO chain key at
      // all, and its `?chain=` is the turn-0 Mote — so a keyed request matches nothing and
      // the run stays exactly as under-rendered as before. Membership finds the right
      // chain in both shapes, and can never select a different run's.
      let roster: readonly string[] = [];
      if (scopeMoteId) {
        try {
          const page = await client.listReactTurns({ instanceId, limit: CHAIN_PAGE });
          const chains = chainAnchors(page.turns);
          const mine =
            chainContaining(chains, scopeMoteId) ??
            (scopeFallbackMoteId ? chainContaining(chains, scopeFallbackMoteId) : null);
          if (mine) {
            const own = new Set(mine.turnMoteIds);
            // The answer commits onto the launch only in the shape where the primary
            // anchor is an admitted Mote OUTSIDE the chain's own turns — the agentic
            // launch step. When the anchor is itself a turn, nothing absorbs.
            const launch =
              vm.motes.some((m) => m.moteId === scopeMoteId) && !own.has(scopeMoteId)
                ? scopeMoteId
                : undefined;
            roster = agenticLineage(
              page.turns.filter((r) => own.has(r.turnMoteId)),
              launch,
            );
          }
        } catch {
          // A gateway without this RPC, or a transient failure. Degrade to the scope
          // this view had before — never fail a projection that would otherwise render.
          roster = [];
        }
      }
      // Scope INSIDE the query fn so every consumer of this hook — the DAG, the mote
      // table, artifacts, metrics, the per-mote detail fan-out — inherits one run's
      // worth of data, and the fan-out shrinks from workspace-sized to run-sized.
      const scoped = scopeProjection(vm, scopeMoteId, scopeFallbackMoteId, roster);
      return roster.length > 0 ? { ...scoped, agenticTurnIds: roster } : scoped;
    },
    refetchInterval: (query) => {
      if (atSeq != null) {
        return false; // a pinned-seq snapshot is static
      }
      const data = query.state.data;
      if (!data) {
        return POLL_MS; // still loading
      }
      const key = `${endpoint}|${instanceId ?? ""}`;
      const f = frontier.current;
      if (f.key !== key) {
        f.key = key; // a different run — reset frontier tracking
        f.lastSeq = -1;
      }
      const prevSeq = f.lastSeq;
      f.lastSeq = data.currentSeq;
      return isRunAtRest(data, terminalMoteId, prevSeq) ? false : POLL_MS;
    },
    refetchIntervalInBackground: false,
  });
}
