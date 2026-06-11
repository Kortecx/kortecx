import { m } from "framer-motion";
import { useState } from "react";
import { stagger } from "../../app/motion";
import { DatasetsPanel } from "../datasets/DatasetsPanel";
import { IngestPanel } from "../datasets/IngestPanel";
import { QueryPanel } from "../datasets/QueryPanel";

/**
 * The Datasets data-plane (RAG) console (T3.7): the corpora the gateway holds, a
 * semantic search over the selected one, and a text-ingest form. Backed by the
 * additive ListDatasets / QueryDataset / IngestDocuments RPCs over an in-process
 * HNSW ANN index (`kx-dataset-hnsw`). Text ingest/search need a server embedder
 * (`kx serve --features inference`); the SDK's FFI-free client-vector path needs
 * none. Retrieval scores are DISPLAY-only (SN-8) — a ranking aid, never identity.
 */
export function DatasetsSection() {
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <section className="screen" data-testid="datasets-section">
      <h1>Datasets</h1>
      <p className="muted">
        Retrieval corpora for grounding agentic runs — ingest documents, then search by meaning.
      </p>
      <m.div className="datasets-grid" variants={stagger()} initial="hidden" animate="show">
        <DatasetsPanel selectedDataset={selected} onSelect={setSelected} />
        <QueryPanel dataset={selected} />
        <IngestPanel />
      </m.div>
    </section>
  );
}
