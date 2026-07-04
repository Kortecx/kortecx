/** TriggersPanel (D113 / D170.b) — the trigger-registry govern surface: the
 *  not-wired / loading / empty / list states, the register form (kind + auth CHIP
 *  groups, conditional secret-ref + schedule fields), and the per-row Test / Fire /
 *  Remove. The kx hooks are mocked → a pure render/interaction check. */

import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const listState = {
  triggers: [] as Array<Record<string, unknown>>,
  notWired: false,
  isLoading: false,
  isError: false,
  error: null as unknown,
  refetch: vi.fn(),
};
const mut = (mutate: ReturnType<typeof vi.fn>) => ({
  mutate,
  isPending: false,
  variables: undefined as unknown,
  error: null as unknown,
  isSuccess: false,
  data: undefined as unknown,
});
const registerM = mut(vi.fn());
const removeM = mut(vi.fn());
const testM = mut(vi.fn());
const fireM = mut(vi.fn());

vi.mock("../../src/kx/use-triggers", () => ({
  useListTriggers: () => listState,
  useRegisterTrigger: () => registerM,
  useDeregisterTrigger: () => removeM,
  useTestTrigger: () => testM,
  useFireTrigger: () => fireM,
}));

import { TriggersPanel } from "../../src/components/tools/TriggersPanel";

function resetMut(m: ReturnType<typeof mut>) {
  m.isPending = false;
  m.variables = undefined;
  m.error = null;
  m.isSuccess = false;
  m.data = undefined;
  m.mutate.mockClear();
}

const oneTrigger = (over: Record<string, unknown> = {}) => {
  listState.triggers = [
    {
      triggerId: "ab".repeat(8),
      name: "gh-push",
      kind: "webhook",
      recipeHandle: "kx/recipes/react",
      appHandle: "",
      auth: "hmac_sha256",
      authSecretPresent: true,
      scheduleSpec: "",
      timezone: "",
      enabled: true,
      requireApproval: false,
      lastFireUnixMs: 0,
      ...over,
    },
  ];
};

afterEach(() => {
  listState.triggers = [];
  listState.notWired = false;
  listState.isLoading = false;
  [registerM, removeM, testM, fireM].forEach(resetMut);
});

describe("TriggersPanel", () => {
  it("shows the honest not-wired empty state", () => {
    listState.notWired = true;
    render(<TriggersPanel />);
    expect(screen.getByText("Triggers not enabled")).toBeInTheDocument();
  });

  it("shows the empty state + the register form when none are registered", () => {
    render(<TriggersPanel />);
    expect(screen.getByText("No triggers registered")).toBeInTheDocument();
    expect(screen.getByTestId("trigger-add-form")).toBeInTheDocument();
    // The kind + auth chip groups are always present.
    expect(screen.getByTestId("trigger-kind-webhook")).toBeInTheDocument();
    expect(screen.getByTestId("trigger-auth-none")).toBeInTheDocument();
  });

  it("renders a registered trigger with its kind/auth/signed indicator + per-row actions", () => {
    oneTrigger();
    render(<TriggersPanel />);
    expect(screen.getByTestId("trigger-gh-push")).toBeInTheDocument();
    expect(screen.getByText("kx/recipes/react")).toBeInTheDocument();
    // authSecretPresent → the lock/"signed" indicator.
    expect(screen.getByTestId("trigger-signed-gh-push")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("trigger-test-gh-push"));
    expect(testM.mutate).toHaveBeenCalledWith({ name: "gh-push" });
    fireEvent.click(screen.getByTestId("trigger-fire-gh-push"));
    expect(fireM.mutate).toHaveBeenCalledWith({ name: "gh-push" });
    fireEvent.click(screen.getByTestId("trigger-remove-gh-push"));
    expect(removeM.mutate).toHaveBeenCalledWith("gh-push");
  });

  it("shows the inline Test (ok) + Fire (instance id) outcomes per row", () => {
    oneTrigger();
    testM.data = { ok: true, detail: "binds to kx/recipes/react" };
    fireM.data = { instanceId: "cd".repeat(8), deduped: false };
    render(<TriggersPanel />);
    expect(screen.getByTestId("trigger-test-result-gh-push")).toHaveTextContent("Binding OK");
    expect(screen.getByTestId("trigger-fire-result-gh-push")).toHaveTextContent("cd".repeat(8));
  });

  it("the cron kind reveals the schedule field; non-none auth reveals the secret-ref field", () => {
    render(<TriggersPanel />);
    // Defaults: webhook + none → neither conditional field is shown.
    expect(screen.queryByTestId("trigger-add-schedule")).toBeNull();
    expect(screen.queryByTestId("trigger-add-secret-ref")).toBeNull();
    // Switch kind → cron via the CHIP control (not a <select>).
    fireEvent.click(screen.getByTestId("trigger-kind-cron"));
    expect(screen.getByTestId("trigger-add-schedule")).toBeInTheDocument();
    // Switch auth → hmac via the CHIP control.
    fireEvent.click(screen.getByTestId("trigger-auth-hmac_sha256"));
    expect(screen.getByTestId("trigger-add-secret-ref")).toBeInTheDocument();
  });

  it("registers a webhook trigger with the chosen chips + fields", () => {
    render(<TriggersPanel />);
    fireEvent.change(screen.getByTestId("trigger-add-name"), { target: { value: "gh-push" } });
    fireEvent.change(screen.getByTestId("trigger-add-recipe"), {
      target: { value: "kx/recipes/react" },
    });
    fireEvent.submit(screen.getByTestId("trigger-add-form"));
    expect(registerM.mutate).toHaveBeenCalledTimes(1);
    const input = registerM.mutate.mock.calls[0]?.[0];
    expect(input).toMatchObject({
      name: "gh-push",
      kind: "webhook",
      recipeHandle: "kx/recipes/react",
      auth: "none",
      enabled: true,
    });
  });

  it("does not register without a name + recipe (and a secret ref when auth≠none)", () => {
    render(<TriggersPanel />);
    fireEvent.change(screen.getByTestId("trigger-add-name"), { target: { value: "gh-push" } });
    // No recipe handle yet → blocked.
    fireEvent.submit(screen.getByTestId("trigger-add-form"));
    expect(registerM.mutate).not.toHaveBeenCalled();
  });

  it("T-APP-TRIGGER-TARGET: registers an App target + HITL + cron/timezone", () => {
    render(<TriggersPanel />);
    // Cron kind reveals the schedule + timezone fields.
    fireEvent.click(screen.getByTestId("trigger-kind-cron"));
    // Swap the target to App (recipe input → app input).
    fireEvent.click(screen.getByTestId("trigger-target-app"));
    expect(screen.queryByTestId("trigger-add-recipe")).toBeNull();
    fireEvent.change(screen.getByTestId("trigger-add-name"), { target: { value: "standup" } });
    fireEvent.change(screen.getByTestId("trigger-add-app"), {
      target: { value: "standup-digest" },
    });
    fireEvent.change(screen.getByTestId("trigger-add-schedule"), {
      target: { value: "0 9 * * 1-5" },
    });
    fireEvent.change(screen.getByTestId("trigger-add-timezone"), {
      target: { value: "America/New_York" },
    });
    fireEvent.click(screen.getByTestId("trigger-add-require-approval"));
    fireEvent.submit(screen.getByTestId("trigger-add-form"));
    expect(registerM.mutate).toHaveBeenCalledTimes(1);
    expect(registerM.mutate.mock.calls[0]?.[0]).toMatchObject({
      name: "standup",
      kind: "cron",
      recipeHandle: "",
      appHandle: "standup-digest",
      scheduleSpec: "0 9 * * 1-5",
      timezone: "America/New_York",
      requireApproval: true,
    });
  });

  it("renders an App-target trigger with the app: target + HITL chip", () => {
    oneTrigger({
      name: "standup",
      kind: "cron",
      recipeHandle: "",
      appHandle: "standup-digest",
      auth: "none",
      authSecretPresent: false,
      scheduleSpec: "0 9 * * 1-5",
      timezone: "America/New_York",
      requireApproval: true,
    });
    render(<TriggersPanel />);
    expect(screen.getByText("app:standup-digest")).toBeInTheDocument();
    expect(screen.getByTestId("trigger-hitl-standup")).toBeInTheDocument();
    expect(screen.getByTestId("trigger-target-kind-standup")).toHaveTextContent("app");
  });
});
