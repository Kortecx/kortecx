import { BranchList } from "../branches/BranchList";
import { NewBranchForm } from "../branches/NewBranchForm";

/**
 * Branches (D155) — named, content-addressed `{path → ref}` manifests over
 * operator-approved host files. Two surfaces over the gateway's branch store:
 *
 * 1. **Your branches (govern/review)** — the durable inventory (`ListBranches`):
 *    every branch this party authored, its `{path → ref}` manifest, the
 *    server-derived `branchRef`, the CoW parent (if a fork), and a delete control
 *    (unbinds the handle; the CAS blobs stay). Caller-scoped (SN-8).
 * 2. **Author** — snapshot a confined path set (`SnapshotInto`) or create/fork a
 *    branch (`CreateBranch`). Files are read server-side from `KX_SERVE_FS_ROOT`;
 *    the host is never written (Phase-A).
 *
 * Both degrade to a not-wired empty state on older gateways (UNIMPLEMENTED).
 * Governed host write-back + cross-party sharing are later / Cloud capabilities.
 */
export function BranchesSection() {
  return (
    <section className="screen" data-testid="branches-section">
      <h1>Branches</h1>
      <p className="muted">
        Snapshot operator-approved host files into content-addressed branches, then let the agent
        loop edit them in the content store. Copy-on-write and governed: a branch is a manifest of
        immutable refs, a sub-branch re-points only changed files, and the host filesystem is never
        written in this phase (SN-8).
      </p>

      <h2>Your branches</h2>
      <p className="muted">
        Every branch you authored, with its <span className="mono">{"{path → ref}"}</span> manifest
        and the server-derived branch ref. Deleting unbinds the handle; the content-store blobs
        stay.
      </p>
      <BranchList />

      <NewBranchForm />
    </section>
  );
}
