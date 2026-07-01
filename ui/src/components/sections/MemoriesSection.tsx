import { m } from "framer-motion";
import { useState } from "react";
import { stagger } from "../../app/motion";
import { useForgetMemory, useMemories, useMemoryRecall, useStoreMemory } from "../../kx/use-memory";

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

/** The episodic memory log (newest-first) with per-item forget. */
function MemoriesListPanel() {
  const memories = useMemories();
  const forget = useForgetMemory();
  return (
    <div className="glow-card" data-testid="memory-list-panel">
      <h2>Your memories</h2>
      <p className="muted">
        Everything remembered, newest-first, scoped to you. Forgetting erases the fact; a later run
        can re-learn it.
      </p>
      {memories.isError ? <NotWired /> : null}
      {memories.data && memories.data.length === 0 ? (
        <p className="muted" data-testid="memory-list-empty">
          (no memories yet)
        </p>
      ) : null}
      <ol className="dataset-hits" data-testid="memory-list">
        {(memories.data ?? []).map((mem) => (
          <li key={mem.memoryId} className="dataset-hit">
            <span className="chip">{mem.kind}</span>
            <span className="dataset-hit__text">{snippet(mem.text)}</span>
            <button
              type="button"
              data-testid={`memory-forget-${mem.memoryId.slice(0, 8)}`}
              disabled={forget.isPending}
              onClick={() => forget.mutate(mem.memoryId)}
            >
              Forget
            </button>
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
      <m.div className="datasets-grid" variants={stagger()} initial="hidden" animate="show">
        <StorePanel />
        <RecallPanel />
        <MemoriesListPanel />
      </m.div>
    </section>
  );
}
