import { m } from "framer-motion";
import { useState } from "react";
import { stagger } from "../../app/motion";
import { DatasetsPanel } from "../datasets/DatasetsPanel";
import { IngestPanel } from "../datasets/IngestPanel";
import { QueryPanel } from "../datasets/QueryPanel";

/** One honest-disabled "Cloud" capability card (GR15 don't-fake-gaps + D157/GR19:
 *  the OSS line ships view/author + deterministic engineering; the managed,
 *  agentic, and analytics halves are the Cloud offering). */
function CloudCard({ label, detail }: { label: string; detail: string }) {
  return (
    <div className="metric-card metric-card--disabled">
      <span className="metric-card__value">
        <span className="chip--soon">Cloud</span>
      </span>
      <span className="metric-card__label">{label}</span>
      <span className="metric-card__sub">{detail}</span>
    </div>
  );
}

/**
 * The Data Lab — the OSS data-plane workbench (T3.7 + D157): the retrieval corpora
 * the gateway holds, a semantic search / advisory discovery over the selected one
 * (rendering hits through the multi-modal {@link AssetViewer}), and a text-ingest
 * form. Backed by the additive ListDatasets / QueryDataset / FuzzyDiscovery /
 * IngestDocuments RPCs over an in-process HNSW ANN index (`kx-dataset-hnsw`). Text
 * ingest/search need a server embedder (`kx serve --features inference`); the SDK's
 * FFI-free client-vector path needs none. Retrieval scores are DISPLAY-only (SN-8).
 *
 * Honest Cloud boundary (D157/GR19): vector retrieval + deterministic synthesis run
 * locally in OSS; LLM-driven synthesis, SQL/transform/visualize, an external
 * (bring-your-own Postgres) database, analytics/dashboards, and governance/lineage
 * are the managed Cloud offering — surfaced as honest-disabled cards, never fakes.
 */
export function DatasetsSection() {
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <section className="screen" data-testid="datasets-section">
      <h1>Data Lab</h1>
      <p className="muted">
        Retrieval corpora for grounding agentic runs — ingest documents, then search by meaning and
        preview hits (text, JSON, markdown, images, audio &amp; video) in the browser.
      </p>
      <m.div className="datasets-grid" variants={stagger()} initial="hidden" animate="show">
        <DatasetsPanel selectedDataset={selected} onSelect={setSelected} />
        <QueryPanel dataset={selected} />
        <IngestPanel />
      </m.div>

      <h2 className="datasets-cloud__title">Data engineering</h2>
      <p className="muted">
        Vector retrieval and deterministic synthesis run locally. Agentic data synthesis, SQL
        transforms, analytics, and managed databases are part of Kortecx Cloud.
      </p>
      <div className="metrics-grid" data-testid="datasets-cloud-disabled">
        <CloudCard
          label="LLM data synthesis"
          detail="Deterministic synthesis runs locally; LLM-driven synthetic-data generation is a Cloud capability."
        />
        <CloudCard
          label="SQL · transform · visualize"
          detail="Query/transform pipelines and chart-grade visualization are a Cloud capability."
        />
        <CloudCard
          label="External database"
          detail="Bring-your-own Postgres / managed multi-modal data layer is a Cloud offering."
        />
        <CloudCard
          label="Analytics & governance"
          detail="Cross-run analytics, dashboards, and data lineage/governance are a Cloud capability."
        />
      </div>
    </section>
  );
}
