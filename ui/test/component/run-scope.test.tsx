/**
 * The run view's SCOPE, end to end at the route boundary: the `?chain=` a navigation
 * produces survives `validateSearch`, and the view renders the honest "this is the whole
 * journal" notice when — and only when — it could not scope itself.
 *
 * This is the test that was missing in #362. The scoping machinery landed with a
 * `scopeMoteId` option, a `scopeMissed` flag and a connected-component walk, all correct
 * and all unreachable, because nothing asserted that an anchor actually reaches the view.
 *
 * `createRoute` is stubbed to the identity so the route module's options object (its
 * `component` + `validateSearch`) is directly testable without mounting the router or the
 * app shell — the established pattern in this suite (`vi.mock("@tanstack/react-router")`).
 */

import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { runViewSearch } from "../../src/lib/run-anchor";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";
import { mote, projection } from "../mocks/projection-fixtures";

const INSTANCE = "ab".repeat(16);
const A1 = "a1".repeat(32);
const A2 = "a2".repeat(32);
const B1 = "b1".repeat(32);

/** The search the mocked `useSearch` hands the view — set per test. */
let SEARCH: Record<string, unknown> = {};

vi.mock("@tanstack/react-router", () => ({
  createRoute: (opts: unknown) => opts,
  useParams: () => ({ instanceId: INSTANCE }),
  useSearch: () => SEARCH,
  useNavigate: () => vi.fn(),
  Link: ({ to, children, ...rest }: Record<string, unknown> & { children?: unknown }) => (
    <a href={typeof to === "string" ? to : "#"} {...rest}>
      {children as never}
    </a>
  ),
}));
// The real root route pulls in the whole app shell; the route under test only needs a
// parent object to exist.
vi.mock("../../src/router/routes/__root", () => ({ rootRoute: {} }));

import { workflowDetailRoute } from "../../src/router/routes/workflow-detail";

/** The route options object (see the `createRoute` stub above). */
const routeOptions = workflowDetailRoute as unknown as {
  component: () => JSX.Element;
  validateSearch: (search: object) => Record<string, unknown>;
};

/** Two unrelated runs in one journal — a `kx serve` shares ONE instance id across all of
 *  them, which is the entire reason the run view needs an anchor. */
function journalClient() {
  return makeMockClient({
    getProjection: async () =>
      projection([
        mote({ moteId: A1, stateCode: 3 }),
        mote({ moteId: A2, stateCode: 3, parents: [{ parentId: A1 }] }),
        mote({ moteId: B1, stateCode: 3 }),
      ]),
  });
}

function renderRun(search: Record<string, unknown>) {
  SEARCH = search;
  const { client } = journalClient();
  const Screen = routeOptions.component;
  return render(<Screen />, { wrapper: connectedWrapper(client) });
}

beforeEach(() => {
  SEARCH = {};
});

describe("the run view's unscoped notice", () => {
  it("a run opened WITH an anchor renders no notice", async () => {
    // `tab: "table"` keeps the assertion on the data layer — the graph tab lazy-loads
    // reactflow, which has nothing to do with scoping.
    renderRun({ tab: "table", chain: A2 });
    await waitFor(() => expect(screen.getByTestId("mote-table")).toBeInTheDocument());
    expect(screen.queryByTestId("run-unscoped-notice")).toBeNull();
  });

  it("and shows ONLY that run's steps, not the journal's", async () => {
    renderRun({ tab: "table", chain: A2 });
    await waitFor(() => expect(screen.getByTestId("mote-table")).toBeInTheDocument());
    const rows = screen.getAllByTestId("mote-row");
    expect(rows).toHaveLength(2); // A1 → A2; B1 belongs to somebody else's run
  });

  it("a run opened WITHOUT an anchor says so", async () => {
    // A hand-typed URL, a durable row recovered from `ListRuns`, a server older than
    // `terminal_mote_id`. The view must not pass the journal off as the run.
    renderRun({ tab: "table" });
    await waitFor(() => expect(screen.getByTestId("run-unscoped-notice")).toBeInTheDocument());
    expect(screen.getAllByTestId("mote-row")).toHaveLength(3);
    expect(screen.getByTestId("run-unscoped-notice").textContent).toContain(
      "Showing every step in this server's journal",
    );
  });

  it("an anchor that is not in the fold says the link may be stale", async () => {
    renderRun({ tab: "table", chain: "ff".repeat(32) });
    await waitFor(() => expect(screen.getByTestId("run-unscoped-notice")).toBeInTheDocument());
    expect(screen.getByTestId("run-unscoped-notice").textContent).toContain("may be stale");
  });
});

describe("the anchor → URL round trip", () => {
  it("what a navigation emits is what the route accepts", () => {
    // The two halves are written in different modules; if `validateSearch`'s shape check
    // and `runViewSearch`'s key names ever drift, scoping dies silently at the URL.
    const search = runViewSearch({ reactChainSalt: A2, terminalMoteId: B1 });
    expect(routeOptions.validateSearch(search)).toEqual({ terminal: B1, chain: A2 });
  });

  it("a salt-less run round-trips its terminal Mote as the anchor", () => {
    const search = runViewSearch({ reactChainSalt: "", terminalMoteId: B1 });
    expect(routeOptions.validateSearch(search)).toEqual({ terminal: B1, chain: B1 });
  });

  it("an unscopable run round-trips to nothing (and the view then notices)", () => {
    expect(routeOptions.validateSearch(runViewSearch({}))).toEqual({});
  });

  it("a malformed anchor is DROPPED, never passed to the fold", () => {
    // A truncated/garbage id would match no Mote and empty the run; refusing it at the
    // boundary keeps the failure in the honest "no anchor" bucket.
    expect(routeOptions.validateSearch({ chain: "not-a-mote-id" })).toEqual({});
  });
});
