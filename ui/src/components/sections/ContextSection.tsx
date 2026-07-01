import type { ContextTab } from "../../router/routes/context";
import { ContextBundleList } from "../context/ContextBundleList";
import { NewContextBundleForm } from "../context/NewContextBundleForm";
import { DatasetsSection } from "./DatasetsSection";
import { MemoriesSection } from "./MemoriesSection";

const TABS: ReadonlyArray<{ id: ContextTab; label: string }> = [
  { id: "bundles", label: "Bundles" },
  { id: "datasets", label: "Datasets" },
  { id: "memories", label: "Memories" },
];

/**
 * Context — the data & storage umbrella (POC-5c / D168). Two URL-addressable tabs
 * over TWO SEPARATE stores (no backend merge — honest, GR15):
 *
 * 1. **Bundles** — named, content-addressed instruction/file bundles a caller attaches
 *    to a run (PR-7, `bundles.db`): the durable inventory (`ListContextBundles`) + an
 *    author form (`PutContextBundle`). Caller-scoped (SN-8). The default tab.
 * 2. **Datasets** — the RAG corpora / Data Lab (`datasets.db`): the existing
 *    {@link DatasetsSection} verbatim (ingest, semantic search, agent outputs).
 *
 * Tab state rides the route's validated search (the run-detail precedent) so the
 * section stays a pure renderer; both surfaces degrade to honest not-wired states.
 */
export function ContextSection({
  tab = "bundles",
  onTab,
}: {
  tab?: ContextTab;
  onTab?: (tab: ContextTab) => void;
} = {}) {
  return (
    <section className="screen" data-testid="context-section">
      <div className="section-head">
        <div>
          <h1>Context</h1>
          <p className="muted">
            The runtime's data &amp; storage: reusable instruction/file bundles you attach to chats
            and chains, and the RAG corpora your agents retrieve from. Two distinct stores under one
            roof — bundles bind to a run's entry step (SN-8), datasets ground a retrieval turn.
          </p>
        </div>
      </div>

      <fieldset className="view-toggle" aria-label="Context view" data-testid="context-tabs">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            data-testid={`context-tab-${t.id}`}
            aria-pressed={tab === t.id}
            onClick={() => onTab?.(t.id)}
          >
            {t.label}
          </button>
        ))}
      </fieldset>

      {tab === "datasets" ? (
        <DatasetsSection />
      ) : tab === "memories" ? (
        <MemoriesSection />
      ) : (
        <>
          <h2>Your bundles</h2>
          <p className="muted">
            Every bundle you authored, with its items and the server-derived bundle ref. Deleting
            unbinds the handle; the content-store blobs stay.
          </p>
          <ContextBundleList />
          <NewContextBundleForm />
        </>
      )}
    </section>
  );
}
