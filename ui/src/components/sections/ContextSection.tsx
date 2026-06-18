import { ContextBundleList } from "../context/ContextBundleList";
import { NewContextBundleForm } from "../context/NewContextBundleForm";

/**
 * Context — named, content-addressed bundles a caller attaches to a run (PR-7) so
 * a model reasons over its grounding. Two surfaces over the gateway's bundle store:
 *
 * 1. **Your bundles (govern/review)** — the durable inventory (`ListContextBundles`):
 *    every bundle this party authored, its items (each a content-store ref), the
 *    server-derived `bundleRef`, and a delete control (unbinds the handle; the CAS
 *    blobs stay). Caller-scoped (SN-8 — no cross-party listing).
 * 2. **Author** — upsert a bundle (`PutContextBundle`) from uploaded files or
 *    existing content refs. Attach it in chat (the composer attach-menu) or a chain
 *    (`kx chain run --context`, `chain(...).context(...)`).
 *
 * Both degrade to a not-wired empty state on older gateways (UNIMPLEMENTED).
 * Cross-party sharing + a bundle marketplace are a Cloud capability (GR19).
 */
export function ContextSection() {
  return (
    <section className="screen" data-testid="context-section">
      <h1>Context</h1>
      <p className="muted">
        Reusable instruction &amp; file bundles you attach to chats and chains. The server resolves
        each bundle to its content refs and folds them into the run's entry step — identity-bearing,
        so a different attached context is a different, independently-cached run (SN-8).
      </p>

      <h2>Your bundles</h2>
      <p className="muted">
        Every bundle you authored, with its items and the server-derived bundle ref. Deleting
        unbinds the handle; the content-store blobs stay.
      </p>
      <ContextBundleList />

      <NewContextBundleForm />
    </section>
  );
}
