import { EmptyState } from "../EmptyState";

/** Forward-compatible placeholder — the RAG/datasets track (DP1→DP3) is unbuilt. */
export function DatasetsSection() {
  return (
    <section className="screen" data-testid="datasets-section">
      <h1>Datasets</h1>
      <p className="muted">Retrieval corpora for grounding agentic runs.</p>
      <EmptyState
        title="Coming soon"
        detail="The datasets / RAG data-plane (embedded LanceDB) is on the DP1→DP3 track and is not built yet."
      />
    </section>
  );
}
