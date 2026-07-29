/**
 * The App KIND/MODE axes as the create surface authors them.
 *
 * Two behaviours, both of which used to be wrong in a way that looked like nothing was
 * wrong:
 *
 * 1. Authoring a HOSTED app from the Scheduled tab used to leave the catalog on
 *    Scheduled — created correctly, but invisible. The form now reports the kind it
 *    actually SAVED under in its launch outcome, and the create screen's terminal
 *    dialog routes home by it.
 * 2. A scheduled app's authoring MODE (contextual vs codified) had no surface at all.
 *
 * Its own file rather than an addition to `apps-create.test.tsx`: proving (1) needs the
 * save mutation to genuinely SUCCEED, so `@kortecx/sdk/web` has to be a working builder
 * stub rather than that file's inert one. The mode CHIP tests still render the catalog.
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

const scaffoldMutate = vi.fn((_vars: unknown, opts?: { onSettled?: () => void }) => {
  opts?.onSettled?.();
});
vi.mock("../../src/kx/use-scaffold-app", () => ({
  useScaffoldApp: () => ({ mutate: scaffoldMutate, isPending: false, error: null }),
  useScaffoldStatus: () => ({ data: undefined, isLoading: true, isError: false }),
  useInvalidateOnScaffoldDone: () => vi.fn(),
}));

/**
 * The design `DeriveApp` hands back. The surface has no name field until a design exists —
 * nothing is creatable before the review — so every authoring assertion below goes through it.
 */
function design(over: Record<string, unknown> = {}) {
  return {
    derived: true,
    name: "My App",
    description: "what it does",
    steps: [],
    edges: [],
    files: [],
    framework: "vite_react",
    tools: {},
    skills: [],
    connections: [],
    datasets: [],
    notices: [],
    ...over,
  };
}
let DESIGN: ReturnType<typeof design> = design();
const deriveMutate = vi.fn((_input: unknown, opts?: { onSuccess?: (d: unknown) => void }) => {
  opts?.onSuccess?.(DESIGN);
});
vi.mock("../../src/kx/use-derive-app", () => ({
  useDeriveApp: () => ({
    mutate: deriveMutate,
    isPending: false,
    error: null,
    data: undefined,
    reset: vi.fn(),
  }),
}));

/**
 * A chainable App-builder stub: every authoring call returns `this`, and `save` resolves a
 * handle. Enough for the form to complete a save, which is what makes the kind-follow
 * assertion real rather than a re-statement of the mock.
 */
const modeCalls: string[] = [];
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
  ]) {
    builder[m] = () => builder;
  }
  builder.mode = (m: string) => {
    modeCalls.push(m);
    return builder;
  };
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
import { NewAppForm } from "../../src/components/sections/NewAppForm";

const onLaunched = vi.fn();

afterEach(() => {
  APPS = [];
  scaffoldMutate.mockClear();
  deriveMutate.mockClear();
  onLaunched.mockClear();
  modeCalls.length = 0;
  DESIGN = design();
});

/** Mount the create form directly (it lives on /apps/create now, not the catalog). */
function mountForm(initialKind: "scheduled" | "hosted" = "scheduled") {
  return render(
    <NewAppForm initialKind={initialKind} onClose={vi.fn()} onLaunched={onLaunched} />,
  );
}

/** Drive the mounted form to a reviewable design. */
function derive(promptText = "summarize the changelog"): void {
  fireEvent.change(screen.getByTestId("new-app-prompt"), { target: { value: promptText } });
  fireEvent.click(screen.getByTestId("new-app-derive"));
}

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

describe("the launch outcome carries the kind an App was actually authored as", () => {
  it("reports HOSTED when a hosted app is authored from a scheduled-initial form", async () => {
    // THE REGRESSION, one layer down: the form's kind toggle is its OWN state, so
    // authoring a hosted app from the Scheduled tab must report "hosted" — the create
    // screen's terminal dialog routes home by this, or the new app lands invisible.
    mountForm("scheduled");
    fireEvent.click(screen.getByTestId("new-app-kind-hosted"));
    fireEvent.change(screen.getByTestId("new-app-prompt"), {
      target: { value: "a landing page" },
    });
    fireEvent.click(screen.getByTestId("new-app-derive"));
    fireEvent.click(screen.getByTestId("new-app-approve"));

    await waitFor(() =>
      expect(onLaunched).toHaveBeenCalledWith(
        expect.objectContaining({ kind: "hosted", handle: "apps/local/my-app" }),
      ),
    );
  });

  it("reports the matching kind (and a null launch error on a clean launch)", async () => {
    mountForm("scheduled");
    derive("summarize");
    fireEvent.click(screen.getByTestId("new-app-approve"));

    await waitFor(() =>
      expect(onLaunched).toHaveBeenCalledWith(
        expect.objectContaining({ kind: "scheduled", launchError: null }),
      ),
    );
  });
});

describe("the authoring-mode toggle", () => {
  it("DEFAULTS TO CODIFIED and switches to Contextual", () => {
    // The default flipped with the chat surface: what a scheduled app should produce is a real
    // project the runtime is orchestrated from, and contextual is now the deliberate choice.
    mountForm("scheduled");
    expect(screen.getByTestId("new-app-mode-codified")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("new-app-mode-contextual")).toHaveAttribute("aria-pressed", "false");
    // The lede follows the mode: the two produce genuinely different things, and a surface
    // that describes only one of them misleads about the other.
    expect(screen.getByTestId("new-app-lede").textContent).toContain("orchestrated from");

    fireEvent.click(screen.getByTestId("new-app-mode-contextual"));
    expect(screen.getByTestId("new-app-mode-contextual")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("new-app-lede").textContent).toContain("reference notes");
  });

  it("saves codified by default, and emits NO mode key when contextual is chosen", async () => {
    // Guards the wiring between the toggle and the envelope, in BOTH directions. `.mode()` is
    // called only for codified — a contextual app must emit no mode key at all, which is what
    // keeps its canonical bytes (and its app_ref) identical to every app authored before the
    // field existed. Asserting only the codified half would pass on a builder that always
    // wrote a mode.
    const { unmount } = mountForm("scheduled");
    derive("reconcile payouts");
    fireEvent.click(screen.getByTestId("new-app-approve"));
    await waitFor(() => expect(modeCalls).toEqual(["codified"]));

    unmount();
    modeCalls.length = 0;
    mountForm("scheduled");
    fireEvent.click(screen.getByTestId("new-app-mode-contextual"));
    fireEvent.change(screen.getByTestId("new-app-prompt"), { target: { value: "reconcile" } });
    fireEvent.click(screen.getByTestId("new-app-derive"));
    fireEvent.click(screen.getByTestId("new-app-approve"));
    await waitFor(() => expect(scaffoldMutate).toHaveBeenCalled());
    expect(modeCalls).toEqual([]);
  });

  it("is not offered on the hosted lane", () => {
    mountForm("hosted");
    expect(screen.queryByTestId("new-app-mode")).toBeNull();
  });
});

describe("the chat surface derives before it creates", () => {
  it("creates NOTHING until the design is approved", () => {
    mountForm("scheduled");
    derive();
    // The design is on screen and reviewable...
    expect(screen.getByTestId("new-app-review")).toBeTruthy();
    expect(screen.getByTestId("new-app-name")).toHaveValue("My App");
    // ...and nothing has been saved or scaffolded. This inversion is the whole point of the
    // surface: the old form saved first and scaffolded second, so an author's first look at
    // what the runtime decided came after an App already existed.
    expect(scaffoldMutate).not.toHaveBeenCalled();
  });

  it("passes the kind and mode the selectors are on to the derive", () => {
    mountForm("scheduled");
    fireEvent.click(screen.getByTestId("new-app-mode-contextual"));
    fireEvent.change(screen.getByTestId("new-app-prompt"), { target: { value: "triage email" } });
    fireEvent.click(screen.getByTestId("new-app-derive"));
    expect(deriveMutate.mock.calls[0]?.[0]).toMatchObject({
      kind: "scheduled",
      mode: "contextual",
      prompt: "triage email",
    });
  });

  it("shows what the design did NOT get, rather than leaving it to be discovered at run", () => {
    DESIGN = design({ notices: ["not attached — outside what this account can fire: gmail/send"] });
    mountForm("scheduled");
    derive();
    expect(screen.getByTestId("new-app-notices").textContent).toContain("gmail/send");
  });

  it("start over discards the design and returns to the prompt", () => {
    mountForm("scheduled");
    derive();
    fireEvent.click(screen.getByTestId("new-app-start-over"));
    expect(screen.queryByTestId("new-app-review")).toBeNull();
    expect(screen.getByTestId("new-app-compose")).toBeTruthy();
  });

  it("a hosted design reviews its FILE PLAN, not a workflow", () => {
    DESIGN = design({ files: [{ path: "src/App.tsx", role: "the root component" }] });
    mountForm("hosted");
    fireEvent.change(screen.getByTestId("new-app-prompt"), { target: { value: "a timer" } });
    fireEvent.click(screen.getByTestId("new-app-derive"));
    expect(screen.getByTestId("new-app-files")).toBeTruthy();
    expect(screen.queryByTestId("new-app-structure")).toBeNull();
    expect(screen.getByTestId("new-app-file-src/App.tsx")).toBeTruthy();
  });
});
