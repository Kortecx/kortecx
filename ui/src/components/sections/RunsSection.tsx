import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useMemo, useState } from "react";
import { stagger } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { useRecipeNames } from "../../kx/use-recipes";
import { useRuns } from "../../kx/use-runs";
import { shortHex } from "../../lib/format";
import { humanizeHandle } from "../../lib/humanize-handle";
import type { RunRecord } from "../../lib/recent-runs";
import { RUN_NAMES_CHANGED_EVENT, loadRunNames } from "../../lib/run-names";
import { EmptyState } from "../EmptyState";
import { RunCard } from "./RunCard";

/** The display shape a Workflows card renders. */
interface RunDisplay {
  /** Headline: local rename > humanized handle > short id. */
  readonly headline: string;
  /** The raw handle (or fingerprint-joined handle), for the secondary chip. */
  readonly rawHandle: string | null;
  /** The client-local custom name (seeds the rename draft), or null. */
  readonly customName: string | null;
}

/**
 * The Workflows home (PR-4.1b): durable runs enumerated from the journal
 * (`ListRuns`) merged with this session's per-invocation records, rendered as
 * bordered CARDS with clean display names (the raw handle stays a secondary
 * chip) + a per-card action menu. Authoring stays in Blueprints (D141.1) — the
 * "New workflow" button LINKS there, never duplicates it.
 */
export function RunsSection() {
  const { endpoint } = useConnection();
  const { runs, clear, notWired, hasMore, loadMore, refresh } = useRuns();
  const recipeNames = useRecipeNames();
  const [filter, setFilter] = useState("");
  const [names, setNames] = useState<Record<string, string>>(() => loadRunNames(endpoint));

  // Stay fresh across rename events + endpoint switches.
  useEffect(() => {
    setNames(loadRunNames(endpoint));
    function onNamesChanged(): void {
      setNames(loadRunNames(endpoint));
    }
    window.addEventListener(RUN_NAMES_CHANGED_EVENT, onNamesChanged);
    return () => window.removeEventListener(RUN_NAMES_CHANGED_EVENT, onNamesChanged);
  }, [endpoint]);

  const fingerprintNames = recipeNames.data ?? {};

  /** The handle that labels a run (session handle > fingerprint join). */
  function handleFor(r: RunRecord): string | null {
    return (
      r.handle ?? (r.recipeFingerprint ? (fingerprintNames[r.recipeFingerprint] ?? null) : null)
    );
  }

  /** Display name precedence: local rename > humanized handle > short id. */
  function displayFor(r: RunRecord): RunDisplay {
    const local = names[r.instanceId];
    const handleName = handleFor(r);
    const customName = local && local.trim() !== "" ? local : null;
    if (customName) {
      return { headline: customName, rawHandle: handleName, customName };
    }
    if (handleName) {
      return { headline: humanizeHandle(handleName), rawHandle: handleName, customName: null };
    }
    return { headline: shortHex(r.instanceId), rawHandle: null, customName: null };
  }

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (q === "") {
      return runs;
    }
    return runs.filter(
      (r) =>
        r.instanceId.toLowerCase().includes(q) ||
        (r.handle ?? "").toLowerCase().includes(q) ||
        (names[r.instanceId] ?? "").toLowerCase().includes(q) ||
        (r.recipeFingerprint ? (fingerprintNames[r.recipeFingerprint] ?? "") : "")
          .toLowerCase()
          .includes(q),
    );
  }, [runs, filter, names, fingerprintNames]);

  return (
    <section className="screen" data-testid="runs-section">
      <div className="screen__head">
        <h1>Workflows</h1>
        <div className="screen__actions">
          {/* D141.1: authoring is OPERATED in Blueprints — this links there. */}
          <Link to="/recipes" className="btnlink" data-testid="new-workflow">
            New workflow →
          </Link>
          <button type="button" className="linkbtn" onClick={refresh}>
            Refresh
          </button>
          {runs.length > 0 ? (
            <button
              type="button"
              className="linkbtn"
              data-testid="clear-local-history"
              title="Clears THIS browser's invocation history. Durable runs live in the journal and stay."
              onClick={() => {
                clear();
                refresh();
              }}
            >
              Clear local history
            </button>
          ) : null}
        </div>
      </div>
      <p className="muted">
        {notWired
          ? "This gateway does not enumerate runs — showing this session's history only."
          : "Durable runs from the journal, merged with this session's invocations. Clearing removes only this browser's history — journal runs stay."}
      </p>

      {runs.length > 0 ? (
        <input
          className="filter-input"
          data-testid="runs-filter"
          placeholder="Filter by name, id or blueprint…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          spellCheck={false}
          autoComplete="off"
          aria-label="Filter runs"
        />
      ) : null}

      {runs.length === 0 ? (
        <EmptyState
          title="No runs yet"
          detail="Start one with “New workflow” — pick a blueprint, fill its inputs, and run it."
        />
      ) : shown.length === 0 ? (
        <EmptyState title="No matching runs" detail="Adjust the filter above." />
      ) : (
        <m.div
          className="card-grid"
          data-testid="run-list"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {shown.map((r) => {
            const d = displayFor(r);
            return (
              <RunCard
                key={`${r.instanceId}:${r.terminalMoteId ?? ""}`}
                run={r}
                headline={d.headline}
                rawHandle={d.rawHandle}
                customName={d.customName}
              />
            );
          })}
        </m.div>
      )}

      {hasMore ? (
        <button type="button" className="linkbtn" data-testid="runs-load-more" onClick={loadMore}>
          Load more
        </button>
      ) : null}
    </section>
  );
}
