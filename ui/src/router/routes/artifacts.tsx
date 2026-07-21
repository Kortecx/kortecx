import { createRoute, redirect } from "@tanstack/react-router";
import { rootRoute } from "./__root";

interface ArtifactsSearch {
  /** Gallery mode (legacy): browse all of this run's committed artifacts. */
  run?: string;
  /** Deep-link mode (legacy): one committed artifact (`instance` + `ref`). */
  instance?: string;
  ref?: string;
}

/**
 * PR-2 route merge (D141.1): Artifacts is a TAB of a run's detail page. Old
 * deep links map onto it — `?run=` opens the run's gallery tab; `?instance=`
 * + `?ref=` focuses the single artifact; a bare visit lands on the run list.
 */
export const artifactsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/artifacts",
  validateSearch: (search: Record<string, unknown>): ArtifactsSearch => {
    const out: ArtifactsSearch = {};
    // A run instance id is a 16-byte (32 hex char) server-derived id.
    if (typeof search.run === "string" && /^[0-9a-f]{32}$/.test(search.run)) {
      out.run = search.run;
    }
    if (typeof search.instance === "string" && /^[0-9a-f]{32}$/.test(search.instance)) {
      out.instance = search.instance;
    }
    // A content ref is a 32-byte (64 hex char) server-derived id.
    if (typeof search.ref === "string" && /^[0-9a-f]{64}$/.test(search.ref)) {
      out.ref = search.ref;
    }
    return out;
  },
  // NOTE: these legacy links predate run scoping and carry no per-run anchor, so the
  // redirect cannot supply `chain` — the run view lands unscoped and says so. Inventing
  // an anchor from the content ref would be a guess (a `ref` is a CONTENT digest, not a
  // Mote id), and a wrong anchor renders an empty run, which is worse than an honest
  // "showing the whole journal".
  beforeLoad: ({ search }) => {
    if (search.instance && search.ref) {
      throw redirect({
        to: "/workflows/$instanceId",
        params: { instanceId: search.instance },
        search: { tab: "artifacts", ref: search.ref },
      });
    }
    if (search.run) {
      throw redirect({
        to: "/workflows/$instanceId",
        params: { instanceId: search.run },
        search: { tab: "artifacts" },
      });
    }
    throw redirect({ to: "/workflows" });
  },
});
