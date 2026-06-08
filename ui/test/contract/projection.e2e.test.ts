// @vitest-environment node
/**
 * Contract test: drive a REAL `kx serve` and prove the UI's data-layer assumptions
 * hold against the actual runtime (echo works, projection carries the states the UI
 * maps, byte-parity with the CLI, and the auth/permission edges surface correctly).
 * Uses the Node client; the browser gRPC-web + CORS path is proven by the Playwright
 * E2E. Gated on a buildable `kx` (KX_BIN in CI).
 */

import { execFileSync } from "node:child_process";
import { KxClient, KxPermissionDenied, KxUnauthenticated } from "@kortecx/sdk/node";
import type { Result } from "@kortecx/sdk/node";
import { afterAll, describe, expect, it } from "vitest";
import { toUiError } from "../../src/kx/errors";
import { toProjectionVM } from "../../src/kx/use-projection";
import {
  ECHO_HANDLE,
  FANOUT_HANDLE,
  authServer,
  devServer,
  findOrBuildKx,
  stopAllServers,
} from "./serve";

function cli(endpoint: string, ...argv: string[]): Record<string, unknown> {
  const out = execFileSync(findOrBuildKx(), [...argv, "--json", "--endpoint", endpoint], {
    encoding: "utf-8",
  });
  return JSON.parse(out);
}

afterAll(async () => {
  await stopAllServers();
});

describe("UI data path against a real kx serve", () => {
  it("invoke echo → projection shows a COMMITTED terminal Mote the table can render", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const result = (await kx.invoke(
      ECHO_HANDLE,
      { topic: "ui-contract" },
      { wait: true },
    )) as Result;
    expect(result.ok).toBe(true);
    const proj = await kx.getProjection(result.instanceId);
    kx.close();

    const vm = toProjectionVM(proj);
    expect(vm.instanceId).toBe(result.instanceId);
    const terminal = vm.motes.find((m) => m.moteId === result.terminalMoteId);
    expect(terminal, "terminal Mote present in projection").toBeTruthy();
    expect(terminal?.stateCode).toBe(3); // COMMITTED
    expect(terminal?.resultRef).toBeTruthy();
    expect(typeof terminal?.ndClass).toBe("number");
  });

  it("echo terminal Mote exposes an empty parents[] (the DAG-edge wire is live end-to-end)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const result = (await kx.invoke(ECHO_HANDLE, { topic: "edges" }, { wait: true })) as Result;
    const proj = await kx.getProjection(result.instanceId);
    kx.close();
    const terminal = toProjectionVM(proj).motes.find((m) => m.moteId === result.terminalMoteId);
    // The SDK→VM `parents` field is present and, for a single-node recipe, empty.
    expect(Array.isArray(terminal?.parents)).toBe(true);
    expect(terminal?.parents).toEqual([]);
  });

  it("fanout-demo → a multi-node projection whose parents[] form a real DAG", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const result = (await kx.invoke(FANOUT_HANDLE, {}, { wait: true })) as Result;
    expect(result.ok).toBe(true);
    const proj = await kx.getProjection(result.instanceId);
    kx.close();

    const vm = toProjectionVM(proj);
    // root + 3 children + gather, all driven to COMMITTED model-free.
    expect(vm.motes).toHaveLength(5);
    expect(vm.motes.every((m) => m.stateCode === 3)).toBe(true);

    const ids = new Set(vm.motes.map((m) => m.moteId));
    const edges = vm.motes.flatMap((m) => m.parents.map((p) => ({ child: m.moteId, ...p })));
    // Referential integrity: every parent id resolves to a node in the projection.
    expect(edges.every((e) => ids.has(e.parentId))).toBe(true);
    // 3 fan-out edges (root→child) + 3 fan-in edges (child→gather) = 6 DATA edges.
    expect(edges).toHaveLength(6);
    expect(edges.every((e) => e.edgeKind === "data")).toBe(true);

    // Exactly one root (no parents) and the terminal gather joins all three children.
    const roots = vm.motes.filter((m) => m.parents.length === 0);
    expect(roots).toHaveLength(1);
    const gather = vm.motes.find((m) => m.moteId === result.terminalMoteId);
    expect(gather?.parents).toHaveLength(3);
  });

  it("byte-parity: the SDK projection equals the CLI projection (the UI reads the same truth)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const result = (await kx.invoke(ECHO_HANDLE, { topic: "parity" }, { wait: true })) as Result;
    const proj = await kx.getProjection(result.instanceId);
    kx.close();
    const cliProj = cli(s.endpoint, "projection", "--instance", result.instanceId);
    expect(proj.toJSON()).toEqual(cliProj);
  });

  it("auth: no token → Unauthenticated → the UI shows a re-auth prompt", async () => {
    const s = await authServer();
    const kx = new KxClient(s.endpoint); // no token
    const err = await kx
      .listSignatures()
      .then(() => null)
      .catch((e: unknown) => e);
    kx.close();
    expect(err).toBeInstanceOf(KxUnauthenticated);
    expect(toUiError(err).kind).toBe("reauth");
  });

  it("auth: the correct token connects", async () => {
    const s = await authServer();
    const kx = new KxClient(s.endpoint, { token: s.token });
    const sigs = await kx.listSignatures();
    kx.close();
    expect(Array.isArray(sigs)).toBe(true);
  });

  it("a bogus instance id is a uniform permission_denied → the UI shows forbidden", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const err = await kx
      .getProjection("00".repeat(16))
      .then(() => null)
      .catch((e: unknown) => e);
    kx.close();
    expect(err).toBeInstanceOf(KxPermissionDenied);
    expect(toUiError(err).kind).toBe("forbidden");
  });

  it("listSignatures returns an array (catalog read path is wired, not UNIMPLEMENTED)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const sigs = await kx.listSignatures();
    kx.close();
    expect(Array.isArray(sigs)).toBe(true);
  });
});
