import { describe, expect, it } from "vitest";
import {
  readConnections,
  readReachInherit,
  readSecretScope,
  readToolGrants,
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
