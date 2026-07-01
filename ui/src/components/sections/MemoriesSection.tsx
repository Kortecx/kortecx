import { m } from "framer-motion";
import { useState } from "react";
import { stagger } from "../../app/motion";
import {
  useApplyDecay,
  useConsolidateMemory,
  useForgetMemory,
  useMemories,
  useMemoryDecay,
  useMemoryRecall,
  useMemoryStats,
  useRestoreMemory,
  useStoreMemory,
} from "../../kx/use-memory";

/** The TTL preset chips (days) for the decay policy. */
const TTL_PRESETS = [30, 90, 180] as const;
/** The salience-floor preset chips (recall count) for the decay policy. */
const ACCESS_PRESETS = [1, 2, 5] as const;

/** A short single-line preview of a memory's text (trimmed). */
function snippet(text: string, max = 120): string {
  const t = text.replace(/\s+/g, " ").trim();
  return t.length > max ? `${t.slice(0, max)}…` : t;
}

/** The not-wired guidance shown when the gateway has memory disabled (UNIMPLEMENTED)
 *  or lacks an embedder (FAILED_PRECONDITION). Honest, don't-fake-gaps (GR15). */
function NotWired() {
  return (
    <p className="notice notice--warn" data-testid="memories-not-wired">
      Memory is not enabled on this gateway. Run <code>kx serve --features inference,hnsw</code>{" "}
      with a model and <code>KX_SERVE_MEMORY=1</code> to remember and recall facts across runs.
    </p>
  );
}

/** Remember a new fact (server-embed + content-addressed). */
function StorePanel() {
  const [text, setText] = useState("");
  const store = useStoreMemory();
  return (
    <div className="glow-card" data-testid="memory-store-panel">
      <h2>Remember a fact</h2>
      <p className="muted">
        Store a durable fact your agents can recall in later runs — content-addressed and idempotent
        (remembering the same fact twice is a no-op).
      </p>
      <textarea
        className="input"
        data-testid="memory-store-input"
        rows={2}
        placeholder="e.g. the project deadline is March 3rd"
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      <button
        type="button"
        data-testid="memory-store-submit"
        disabled={text.trim().length === 0 || store.isPending}
        onClick={() => {
          store.mutate(text.trim(), { onSuccess: () => setText("") });
        }}
      >
        {store.isPending ? "Remembering…" : "Remember"}
      </button>
      {store.isError ? <NotWired /> : null}
      {store.data ? (
        <p className="muted" data-testid="memory-store-result">
          {store.data.inserted ? "Remembered" : "Already remembered (deduped)"}.
        </p>
      ) : null}
    </div>
  );
}

/** Recall the top-k most-similar memories. */
function RecallPanel() {
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const recall = useMemoryRecall(submitted, 5);
  return (
    <div className="glow-card" data-testid="memory-recall-panel">
      <h2>Recall</h2>
      <p className="muted">Find the memories most relevant to a query (semantic search).</p>
      <form
        className="dataset-query-form"
        onSubmit={(e) => {
          e.preventDefault();
          setSubmitted(query.trim());
        }}
      >
        <input
          type="text"
          className="input"
          data-testid="memory-recall-input"
          placeholder="what do I know about…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Recall query"
        />
        <button
          type="submit"
          data-testid="memory-recall-submit"
          disabled={query.trim().length === 0}
        >
          Recall
        </button>
      </form>
      {recall.isError ? <NotWired /> : null}
      {recall.data && recall.data.length === 0 ? (
        <p className="muted" data-testid="memory-recall-empty">
          (no relevant memories)
        </p>
      ) : null}
      <ol className="dataset-hits" data-testid="memory-recall-hits">
        {(recall.data ?? []).map((h) => (
          <li key={h.memoryId} className="dataset-hit">
            <span className="dataset-hit__score">{h.score.toFixed(3)}</span>
            <span className="dataset-hit__text">{snippet(h.text)}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}

/** RC5b: a compact statistics strip (counts by kind, decayed, dim, embed fingerprint). */
function StatsStrip() {
  const stats = useMemoryStats();
  if (stats.isError) {
    return <NotWired />;
  }
  const s = stats.data;
  return (
    <div className="glow-card" data-testid="memory-stats-strip">
      <h2>Memory stats</h2>
      {s ? (
        <p className="muted">
          <span className="chip">{s.total} live</span>{" "}
          <span className="chip">{s.semantic} semantic</span>{" "}
          <span className="chip">{s.episodic} episodic</span>{" "}
          <span className="chip">{s.tombstoned} decayed</span>{" "}
          <span className="chip">dim {s.dim}</span>{" "}
          <span className="chip">fp {s.embedFingerprint || "(none)"}</span>
        </p>
      ) : (
        <p className="muted">Loading…</p>
      )}
    </div>
  );
}

/** RC5b: preview + apply a reversible TTL+salience decay sweep (CHIP-driven policy —
 *  never a controlled <select>; Playwright can't drive it). */
function DecayPanel() {
  const [ttlDays, setTtlDays] = useState<number>(90);
  const [minAccess, setMinAccess] = useState<number>(1);
  const [previewOn, setPreviewOn] = useState(false);
  const preview = useMemoryDecay(ttlDays, minAccess, previewOn);
  const apply = useApplyDecay();
  return (
    <div className="glow-card" data-testid="memory-decay-panel">
      <h2>Decay</h2>
      <p className="muted">
        Age out stale, rarely-recalled memories — reversible soft-tombstones (never a hard delete;
        restore any time). A memory decays only if it is BOTH older than the TTL AND recalled fewer
        than the salience floor.
      </p>
      <div className="chip-row" data-testid="memory-decay-ttl">
        <span className="muted">TTL (days):</span>
        {TTL_PRESETS.map((d) => (
          <button
            type="button"
            key={d}
            className={ttlDays === d ? "chip chip--active" : "chip"}
            data-testid={`memory-decay-ttl-${d}`}
            onClick={() => setTtlDays(d)}
          >
            {d}
          </button>
        ))}
      </div>
      <div className="chip-row" data-testid="memory-decay-access">
        <span className="muted">Min recalls to keep:</span>
        {ACCESS_PRESETS.map((a) => (
          <button
            type="button"
            key={a}
            className={minAccess === a ? "chip chip--active" : "chip"}
            data-testid={`memory-decay-access-${a}`}
            onClick={() => setMinAccess(a)}
          >
            {a}
          </button>
        ))}
      </div>
      <button type="button" data-testid="memory-decay-preview" onClick={() => setPreviewOn(true)}>
        Preview
      </button>
      {preview.isError ? <NotWired /> : null}
      {preview.data ? (
        <>
          <p className="muted" data-testid="memory-decay-summary">
            Would evict {preview.data.wouldEvict} (keep {preview.data.kept}).
          </p>
          <ol className="dataset-hits" data-testid="memory-decay-candidates">
            {preview.data.candidates.map((c) => (
              <li
                key={c.memoryId}
                className="dataset-hit"
                data-testid={`memory-decay-candidate-${c.memoryId.slice(0, 8)}`}
              >
                <span className="chip">age {c.ageDays}d</span>
                <span className="chip">acc {c.accessCount}</span>
                <span className="dataset-hit__text">{snippet(c.text)}</span>
              </li>
            ))}
          </ol>
          {preview.data.wouldEvict > 0 ? (
            <button
              type="button"
              data-testid="memory-decay-apply"
              disabled={apply.isPending}
              onClick={() => apply.mutate({ ttlDays, minAccess })}
            >
              {apply.isPending ? "Evicting…" : `Evict ${preview.data.wouldEvict} (reversible)`}
            </button>
          ) : null}
          {apply.data ? (
            <p className="muted" data-testid="memory-decay-applied">
              Evicted {apply.data.evicted} — restore any from the decayed view below.
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

/** RC5b: trigger a consolidation chain (distill recent episodics into one durable fact). */
function ConsolidatePanel() {
  const [query, setQuery] = useState("");
  const consolidate = useConsolidateMemory();
  return (
    <div className="glow-card" data-testid="memory-consolidate-panel">
      <h2>Consolidate</h2>
      <p className="muted">
        Distill your recent episodic memories into ONE durable semantic fact. Runs a react-memory
        chain (needs a served model): bundle → distill → remember.
      </p>
      <input
        type="text"
        className="input"
        data-testid="memory-consolidate-query"
        placeholder="optional focus (e.g. the Q3 launch)"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        aria-label="Consolidation focus"
      />
      <button
        type="button"
        data-testid="memory-consolidate-trigger"
        disabled={consolidate.isPending}
        onClick={() => consolidate.mutate({ query: query.trim() || undefined })}
      >
        {consolidate.isPending ? "Consolidating…" : "Consolidate"}
      </button>
      {consolidate.isError ? <NotWired /> : null}
      {consolidate.data ? (
        <p className="muted" data-testid="memory-consolidate-result">
          {consolidate.data.text ? snippet(consolidate.data.text, 240) : "Consolidated."}
        </p>
      ) : null}
    </div>
  );
}

/** The episodic memory log (newest-first) with per-item forget + a decayed view + restore. */
function MemoriesListPanel() {
  const [showDecayed, setShowDecayed] = useState(false);
  const memories = useMemories(showDecayed);
  const forget = useForgetMemory();
  const restore = useRestoreMemory();
  return (
    <div className="glow-card" data-testid="memory-list-panel">
      <h2>Your memories</h2>
      <p className="muted">
        Everything remembered, newest-first, scoped to you. Forgetting erases the fact; a later run
        can re-learn it. Toggle the decayed view to restore aged-out memories.
      </p>
      <div className="chip-row">
        <button
          type="button"
          className={showDecayed ? "chip chip--active" : "chip"}
          data-testid="memory-list-toggle-decayed"
          onClick={() => setShowDecayed((v) => !v)}
        >
          {showDecayed ? "Showing decayed" : "Show decayed"}
        </button>
      </div>
      {memories.isError ? <NotWired /> : null}
      {memories.data && memories.data.length === 0 ? (
        <p className="muted" data-testid="memory-list-empty">
          (no memories yet)
        </p>
      ) : null}
      <ol className="dataset-hits" data-testid="memory-list">
        {(memories.data ?? []).map((mem) => (
          <li
            key={mem.memoryId}
            className={mem.isDecayed ? "dataset-hit dataset-hit--muted" : "dataset-hit"}
          >
            <span className="chip">{mem.kind}</span>
            {mem.isDecayed ? <span className="chip">decayed</span> : null}
            <span className="dataset-hit__text">{snippet(mem.text)}</span>
            {mem.isDecayed ? (
              <button
                type="button"
                data-testid={`memory-restore-${mem.memoryId.slice(0, 8)}`}
                disabled={restore.isPending}
                onClick={() => restore.mutate(mem.memoryId)}
              >
                Restore
              </button>
            ) : (
              <button
                type="button"
                data-testid={`memory-forget-${mem.memoryId.slice(0, 8)}`}
                disabled={forget.isPending}
                onClick={() => forget.mutate(mem.memoryId)}
              >
                Forget
              </button>
            )}
          </li>
        ))}
      </ol>
    </div>
  );
}

/**
 * Memories — the durable agentic MEMORY workbench (RC5a): remember facts, recall them
 * by meaning, and browse the per-principal episodic log. Backed by the additive
 * StoreMemory / RecallMemory / ListMemories / ForgetMemory RPCs over a rebuildable
 * `memory.db` sidecar. Text store/recall need a server embedder (`inference`) + memory
 * enabled (`KX_SERVE_MEMORY=1`); every recall score is DISPLAY-only (SN-8). A gateway
 * without memory degrades to an honest not-wired state (GR15).
 */
export function MemoriesSection() {
  return (
    <section className="screen" data-testid="memories-section">
      <h1>Memories</h1>
      <p className="muted">
        Durable, cross-run agent memory — what your agents learn in one run and recall in the next.
        Scoped to you; forgettable; provable (every recall is a committed fact).
      </p>
      <StatsStrip />
      <m.div className="datasets-grid" variants={stagger()} initial="hidden" animate="show">
        <StorePanel />
        <RecallPanel />
        <ConsolidatePanel />
        <DecayPanel />
        <MemoriesListPanel />
      </m.div>
    </section>
  );
}
