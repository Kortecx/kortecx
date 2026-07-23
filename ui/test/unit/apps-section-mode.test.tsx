/**
 * The App KIND/MODE axes as the catalog surfaces them.
 *
 * Two behaviours, both of which used to be wrong in a way that looked like nothing was
 * wrong:
 *
 * 1. Authoring a HOSTED app from the Scheduled tab left the catalog on Scheduled. The app
 *    was created correctly, but landed in the section the user was not looking at — which
 *    reads as the kind selection being dropped by the scaffold transition. The form now
 *    reports the kind it actually SAVED under, and the catalog follows it.
 * 2. A scheduled app's authoring MODE (contextual vs codified) had no surface at all.
 *
 * Its own file rather than an addition to `apps-section.test.tsx`: proving (1) needs the
 * save mutation to genuinely SUCCEED, so `@kortecx/sdk/web` has to be a working builder
 * stub rather than that file's inert one.
 */

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render as rtlRender, screen, waitFor } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

function render(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return rtlRender(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

interface TestApp {
  handle: string;
  appRef: string;
  name: string;
  version: string;
  description: string;
  tags: string[];
  stepCount: number;
  locked: boolean;
  kind?: string;
  mode?: string;
}

let APPS: TestApp[] = [];

vi.mock("../../src/kx/use-apps", () => ({
  useApps: () => ({
    apps: APPS,
    notWired: false,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useApp: () => ({ data: null, isLoading: false, error: null }),
  useRunApp: () => ({ mutate: vi.fn(), isPending: false, error: null, reset: vi.fn() }),
  useExportAppBundle: () => ({ mutate: vi.fn(), isPending: false, error: null, reset: vi.fn() }),
  useImportApp: () => ({ mutate: vi.fn(), isPending: false, error: null, reset: vi.fn() }),
  useCloneApp: () => ({ mutate: vi.fn(), isPending: false, error: null, reset: vi.fn() }),
  useDeleteApp: () => ({ mutate: vi.fn(), isPending: false, error: null, reset: vi.fn() }),
  // The scheduled lane mounts the (lazy) builder canvas, which pulls this from the same
  // module — so the mock has to cover it or the canvas throws mid-render.
  useSaveApp: () => ({ mutate: vi.fn(), isPending: false, error: null, reset: vi.fn() }),
}));

// A non-null client so the form's save mutation reaches the SDK stub below.
vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => ({ client: {}, endpoint: "http://127.0.0.1:50151" }),
}));

const scaffoldMutate = vi.fn();
vi.mock("../../src/kx/use-scaffold-app", () => ({
  useScaffoldApp: () => ({ mutate: scaffoldMutate, isPending: false, error: null }),
  useScaffoldStatus: () => ({ data: undefined, isLoading: true, isError: false }),
  useInvalidateOnScaffoldDone: () => vi.fn(),
}));

/**
 * A chainable App-builder stub: every authoring call returns `this`, and `save` resolves a
 * handle. Enough for the form to complete a save, which is what makes the kind-follow
 * assertion real rather than a re-statement of the mock.
 */
vi.mock("@kortecx/sdk/web", () => {
  const builder: Record<string, unknown> = {};
  for (const m of [
    "describe",
    "hosted",
    "blueprint",
    "rule",
    "steer",
    "context",
    "dataset",
    "useTool",
    "skill",
    "withConnection",
    "mode",
  ]) {
    builder[m] = () => builder;
  }
  builder.save = () => Promise.resolve({ handle: "apps/local/my-app" });
  return {
    app: () => builder,
    flow: () => ({ agent: () => ({}) }),
    defaultHandle: (n: string) => `apps/local/${n}`,
    Reach: { InheritPrincipal: 1 },
    minimalAppEnvelope: () => ({ schema: "kortecx.app/v1" }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  Link: ({ children, to, params, search, ...rest }: Record<string, unknown>) => (
    <a {...(rest as Record<string, unknown>)}>{children as never}</a>
  ),
}));

// The scheduled lane embeds the @xyflow builder canvas, which needs a `ResizeObserver`
// jsdom does not provide. Stub the canvas here rather than polyfilling globally: nothing
// else in the suite mounts it, and this file is about which SECTION the catalog lands on.
vi.mock("../../src/components/sections/BlueprintBuilderSection", () => ({
  BlueprintBuilderSection: () => <div data-testid="builder-canvas-stub" />,
}));

import { AppsSection, modeHint, modeLabel } from "../../src/components/sections/AppsSection";

afterEach(() => {
  APPS = [];
  scaffoldMutate.mockReset();
});

function scheduledApp(over: Partial<TestApp> = {}): TestApp {
  return {
    handle: "apps/local/notes",
    appRef: "ab".repeat(16),
    name: "Notes",
    version: "1",
    description: "",
    tags: [],
    stepCount: 2,
    locked: false,
    kind: "functional",
    ...over,
  };
}

describe("the authoring-mode chip", () => {
  it("reads Contextual when the app declares no mode", () => {
    // An app saved before the field existed, and an old server that does not send it, must
    // BOTH read as contextual — the same default the runtime applies, so the chip can never
    // claim something the run would not do.
    APPS = [scheduledApp({ mode: undefined })];
    render(<AppsSection />);
    expect(screen.getByTestId("app-mode-apps/local/notes")).toHaveTextContent("Contextual");
  });

  it("reads Codified when the app declares it", () => {
    APPS = [scheduledApp({ mode: "codified" })];
    render(<AppsSection />);
    expect(screen.getByTestId("app-mode-apps/local/notes")).toHaveTextContent("Codified");
  });

  it("is absent on a hosted app, which has no such axis", () => {
    APPS = [scheduledApp({ handle: "apps/local/site", kind: "experience" })];
    render(<AppsSection section="hosted" />);
    expect(screen.queryByTestId("app-mode-apps/local/site")).toBeNull();
  });

  it("labels and hints map the empty string to contextual", () => {
    expect(modeLabel("")).toBe("Contextual");
    expect(modeLabel("codified")).toBe("Codified");
    expect(modeHint("")).toContain("text app");
    expect(modeHint("codified")).toContain("code and configuration");
  });
});

describe("the catalog follows the kind an App was authored as", () => {
  it("switches to Hosted when a hosted app is authored from the Scheduled tab", async () => {
    const onSection = vi.fn();
    render(<AppsSection section="scheduled" onSection={onSection} />);

    fireEvent.click(screen.getByTestId("new-app"));
    fireEvent.click(screen.getByTestId("new-app-kind-hosted"));
    fireEvent.change(screen.getByTestId("new-app-name"), { target: { value: "My App" } });
    fireEvent.change(screen.getByTestId("new-app-goal"), { target: { value: "a landing page" } });
    fireEvent.click(screen.getByTestId("new-app-submit"));

    // THE REGRESSION: without this the catalog stayed on Scheduled and the new hosted app
    // was invisible — created, but filtered out of the only section on screen.
    await waitFor(() => expect(onSection).toHaveBeenCalledWith("hosted"));
  });

  it("leaves the section alone when the authored kind already matches", async () => {
    const onSection = vi.fn();
    render(<AppsSection section="scheduled" onSection={onSection} />);

    fireEvent.click(screen.getByTestId("new-app"));
    fireEvent.change(screen.getByTestId("new-app-name"), { target: { value: "My App" } });
    fireEvent.change(screen.getByTestId("new-app-goal"), { target: { value: "summarize" } });
    fireEvent.click(screen.getByTestId("new-app-submit"));

    await waitFor(() => expect(onSection).toHaveBeenCalledWith("scheduled"));
  });
});

describe("the authoring-mode toggle", () => {
  it("offers Codified as honest-disabled until the codified scaffold rail exists", () => {
    render(<AppsSection section="scheduled" />);
    fireEvent.click(screen.getByTestId("new-app"));
    // Discoverable, but not selectable: the envelope carries the field and the catalog
    // reads it, while the scheduled scaffold still authors markdown only. Saving `codified`
    // today would put a "Codified" chip on an app that is prose.
    expect(screen.getByTestId("new-app-mode-contextual")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("new-app-mode-codified")).toBeDisabled();
  });

  it("is not offered on the hosted lane", () => {
    render(<AppsSection section="hosted" />);
    fireEvent.click(screen.getByTestId("new-app"));
    expect(screen.queryByTestId("new-app-mode")).toBeNull();
  });
});
