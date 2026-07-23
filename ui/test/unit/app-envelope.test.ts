import { describe, expect, it } from "vitest";
import {
  bindingTargets,
  readConnections,
  readReachInherit,
  readSecretScope,
  readToolGrants,
  unbindFromSteps,
  writeConnections,
  writeTools,
} from "../../src/lib/app-envelope";

type Env = Record<string, unknown>;
const refs = (e: Env) => e.references as Env | undefined;
const steer = (e: Env) => e.steering_config as Env | undefined;

describe("app-envelope editors", () => {
  it("attaches a tool grant, mirrors references.tools, preserves siblings", () => {
    const env: Env = { name: "X", references: { skills: [{ name: "s" }] } };
    const next = writeTools(env, { "fs.read": "1" }, false);
    expect(readToolGrants(next)).toEqual({ "fs.read": "1" });
    expect(refs(next)?.tools).toEqual([{ tool_id: "fs.read", tool_version: "1" }]);
    expect(refs(next)?.skills).toEqual([{ name: "s" }]);
    expect(next.name).toBe("X");
  });

  it("detaching the last tool omits steering_config.tools and references entirely", () => {
    const env = writeTools({ name: "X" }, { "fs.read": "1" }, false);
    const next = writeTools(env, {}, false);
    expect(readToolGrants(next)).toEqual({});
    expect(next.references).toBeUndefined();
    expect(next.steering_config).toBeUndefined();
  });

  it("reach=inherit serializes and round-trips even with no grants", () => {
    const next = writeTools({ name: "X" }, {}, true);
    expect(readReachInherit(next)).toBe(true);
    expect(readToolGrants(next)).toEqual({});
    expect((steer(next)?.tools as Env).reach).toBe("inherit_principal");
    expect((steer(next)?.tools as Env).requested_grants).toBeUndefined();
  });

  it("writeTools preserves an existing guards block (orthogonal to tools)", () => {
    const env: Env = { steering_config: { guards: { cost_ceiling: 5 } } };
    const next = writeTools(env, { t: "1" }, false);
    expect(steer(next)?.guards).toEqual({ cost_ceiling: 5 });
    expect((steer(next)?.tools as Env).requested_grants).toEqual({ t: "1" });
  });

  it("attaches a connector, scoping its credential name; detach prunes the scope", () => {
    const attached = writeConnections({ name: "X" }, [
      { descriptor: "gmail", credential_ref: "GMAIL_TOKEN" },
    ]);
    expect(readConnections(attached)).toEqual([
      { descriptor: "gmail", credential_ref: "GMAIL_TOKEN" },
    ]);
    expect(readSecretScope(attached)).toEqual(["GMAIL_TOKEN"]);
    const detached = writeConnections(attached, []);
    expect(readConnections(detached)).toEqual([]);
    expect(readSecretScope(detached)).toEqual([]);
    expect(detached.references).toBeUndefined();
  });

  it("a connector with no credential adds no secret scope; shared creds dedupe", () => {
    const noCred = writeConnections({}, [{ descriptor: "http://x", credential_ref: "" }]);
    expect(readSecretScope(noCred)).toEqual([]);
    const shared = writeConnections({}, [
      { descriptor: "a", credential_ref: "TOK" },
      { descriptor: "b", credential_ref: "TOK" },
    ]);
    expect(readSecretScope(shared)).toEqual(["TOK"]);
  });
});

describe("per-node binding read/scrub", () => {
  const env = (): Env => ({
    blueprint: {
      steps: [
        { prompt: "gather escalations", skills: ["triage"], datasets: ["support"] },
        { prompt: "write the summary" },
      ],
    },
    references: { skills: [{ name: "triage" }] },
  });

  it("bindingTargets names the bound step, and 'the entry step' when unbound", () => {
    expect(bindingTargets(env(), "skills", "triage")).toEqual(["gather escalations"]);
    // Case-insensitive on the declared name — the same rule the runtime resolves by.
    expect(bindingTargets(env(), "skills", "TRIAGE")).toEqual(["gather escalations"]);
    // A declaration no step names binds where it always did.
    expect(bindingTargets(env(), "skills", "unused")).toEqual(["the entry step"]);
  });

  it("bindingTargets falls back to a step ordinal when the step has no prompt", () => {
    const e: Env = { blueprint: { steps: [{ skills: ["s"] }] } };
    expect(bindingTargets(e, "skills", "s")).toEqual(["step 1"]);
  });

  it("unbindFromSteps removes the name and drops the emptied key, leaving siblings", () => {
    const scrubbed = unbindFromSteps(env(), "skills", "triage");
    const steps = (scrubbed.blueprint as { steps: Record<string, unknown>[] }).steps;
    expect(steps[0]).not.toHaveProperty("skills");
    // A sibling axis on the same step is untouched.
    expect(steps[0]?.datasets).toEqual(["support"]);
    // references is not this function's concern — only the step bindings.
    expect((scrubbed.references as { skills: unknown[] }).skills).toHaveLength(1);
  });

  it("unbindFromSteps returns the env unchanged when nothing bound the name", () => {
    const e = env();
    expect(unbindFromSteps(e, "skills", "never-bound")).toBe(e);
    // ...including a hosted App with no blueprint.
    const hosted: Env = { references: { skills: [{ name: "triage" }] } };
    expect(unbindFromSteps(hosted, "skills", "triage")).toBe(hosted);
  });
});
