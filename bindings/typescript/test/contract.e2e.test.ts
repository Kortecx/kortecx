/**
 * Contract / conformance tests against a real `kx serve` (ports the Python
 * `test_contract_e2e.py`). The headline proves **byte-parity**: the SDK's
 * `invoke(..., { wait: true })` and the reference `kx` CLI, hitting the same
 * gateway with the same recipe + args, produce identical server-derived ids and
 * result (SN-8 holds across both surfaces). The rest exercise the
 * projection/content/events flow and the edge cases a mature agentic runtime must
 * handle — including the large-payload and concurrent-client cases.
 */

import { execFileSync } from "node:child_process";
import { afterEach, describe, expect, it } from "vitest";
import {
  KxClient,
  KxInvalidArgument,
  KxNotFound,
  KxPermissionDenied,
  KxUnauthenticated,
  KxWaitTimeout,
} from "../src/node.js";
import type { Delta, Result, Run } from "../src/node.js";
import {
  ECHO_HANDLE,
  authServer,
  devServer,
  findOrBuildKx,
  stopAllServers,
} from "./fixtures/serve.js";

// Content-derived (server-derived, SN-8) fields — identical across any server,
// language, or process. `instance_id` is per-journal-registration, so it agrees
// only WITHIN one server (proven by the read-back test).
const DETERMINISTIC = ["terminal_mote_id", "result_ref", "result_hex", "result_len", "state"];

function cli(endpoint: string, ...argv: string[]): Record<string, unknown> {
  const out = execFileSync(findOrBuildKx(), [...argv, "--json", "--endpoint", endpoint], {
    encoding: "utf-8",
  });
  return JSON.parse(out);
}

afterEach(async () => {
  await stopAllServers();
});

// --- the headline: SDK ⇄ CLI conformance -------------------------------------

describe("SDK ⇄ CLI conformance", () => {
  it("invoke→wait matches the CLI field-for-field", async () => {
    const [sSdk, sCli] = await Promise.all([devServer(), devServer()]);
    const kx = new KxClient(sSdk.endpoint);
    const sdk = (await kx.invoke(ECHO_HANDLE, { topic: "hello" }, { wait: true })) as Result;
    kx.close();
    const cliOut = cli(
      sCli.endpoint,
      "invoke",
      ECHO_HANDLE,
      "--args",
      '{"topic":"hello"}',
      "--wait",
    );
    expect(sdk.ok).toBe(true);
    expect(new Set(Object.keys(sdk.toJSON()))).toEqual(new Set(Object.keys(cliOut)));
    for (const k of DETERMINISTIC) {
      expect(sdk.toJSON()[k], `field ${k} differs SDK vs CLI`).toEqual(cliOut[k]);
    }
  });

  it("SDK and CLI read the same committed result (incl. instance_id)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const result = (await kx.invoke(ECHO_HANDLE, { topic: "readback" }, { wait: true })) as Result;
    const proj = await kx.getProjection(result.instanceId);
    kx.close();
    const cliProj = cli(s.endpoint, "projection", "--instance", result.instanceId);
    expect(proj.toJSON()).toEqual(cliProj);
    const cliContent = cli(
      s.endpoint,
      "content",
      "--ref",
      result.resultRef as string,
      "--instance",
      result.instanceId,
    );
    expect(cliContent.payload_hex).toBe(result.toJSON().result_hex);
  });
});

// --- wait strategies + idempotency -------------------------------------------

describe("wait strategies + idempotency", () => {
  it("poll and events wait-modes agree", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const run = (await kx.invoke(ECHO_HANDLE, { topic: "evt" })) as Run;
    const polled = await run.wait({ mode: "poll" });
    const evented = await run.wait({ mode: "events" });
    kx.close();
    expect(polled.toJSON()).toEqual(evented.toJSON());
  });

  it("idempotent re-invoke → same instance/terminal/ref (the #137 regression class)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const a = (await kx.invoke(ECHO_HANDLE, { topic: "same" }, { wait: true })) as Result;
    const b = (await kx.invoke(ECHO_HANDLE, { topic: "same" }, { wait: true })) as Result;
    kx.close();
    expect(a.instanceId).toBe(b.instanceId);
    expect(a.terminalMoteId).toBe(b.terminalMoteId);
    expect(a.resultRef).toBe(b.resultRef);
  });

  it("distinct args → distinct terminal, same instance", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const a = (await kx.invoke(ECHO_HANDLE, { topic: "alpha" }, { wait: true })) as Result;
    const b = (await kx.invoke(ECHO_HANDLE, { topic: "beta" }, { wait: true })) as Result;
    kx.close();
    expect(a.instanceId).toBe(b.instanceId); // one run instance per recipe
    expect(a.terminalMoteId).not.toBe(b.terminalMoteId); // distinct Mote per input
  });

  it("determinism across fresh servers (SN-8)", async () => {
    const [s1, s2] = await Promise.all([devServer(), devServer()]);
    const a = new KxClient(s1.endpoint);
    const b = new KxClient(s2.endpoint);
    const ra = (await a.invoke(ECHO_HANDLE, { topic: "det" }, { wait: true })) as Result;
    const rb = (await b.invoke(ECHO_HANDLE, { topic: "det" }, { wait: true })) as Result;
    a.close();
    b.close();
    expect(ra.terminalMoteId).toBe(rb.terminalMoteId);
    expect(ra.resultRef).toBe(rb.resultRef);
    expect(ra.bytes).toEqual(rb.bytes);
  });
});

// --- projection → content + events flow --------------------------------------

describe("projection / content / events", () => {
  it("run handle → projection → content", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const run = (await kx.invoke(ECHO_HANDLE, { topic: "flow" })) as Run;
    const result = await run.wait({ timeoutMs: 30_000 });
    expect(result.ok).toBe(true);
    const proj = await run.projection();
    expect(proj.instanceId).toBe(run.instanceId);
    const terminal = proj.mote(run.terminalMoteId);
    expect(terminal?.state).toBe("COMMITTED");
    const payload = await run.content(terminal?.resultRef as string);
    kx.close();
    expect(payload).toEqual(result.bytes);
  });

  it("stream events snapshot sees the terminal commit", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const run = (await kx.invoke(ECHO_HANDLE, { topic: "events" })) as Run;
    await run.wait({ timeoutMs: 30_000 });
    const deltas: Delta[] = [];
    for await (const d of run.events({ since: 0n, follow: false })) {
      deltas.push(d);
    }
    kx.close();
    const committed = deltas.filter((d) => d.kind === "committed");
    expect(committed.some((d) => d.moteId === run.terminalMoteId)).toBe(true);
  });

  it("the WS bridge sees the terminal commit (R5)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const run = (await kx.invoke(ECHO_HANDLE, { topic: "ws" }, { wait: true })) as Result;
    let found = false;
    let i = 0;
    for await (const d of kx.wsEvents(run.instanceId, { since: 0n, wsEndpoint: s.wsEndpoint })) {
      if (d.kind === "committed" && d.moteId === run.terminalMoteId) {
        found = true;
        break;
      }
      if (i++ > 200) break; // the catch-up replay carries it in the first frame
    }
    kx.close();
    expect(found).toBe(true);
  });
});

// --- edge cases a mature runtime must hold -----------------------------------

describe("edge cases", () => {
  it("ownership rejection is a uniform permission_denied (no existence oracle)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const run = (await kx.invoke(ECHO_HANDLE, { topic: "owned" }, { wait: true })) as Result;
    const bogus = "00".repeat(16);
    await expect(kx.getProjection(bogus)).rejects.toBeInstanceOf(KxPermissionDenied);
    // right ref, wrong ownership ticket → uniform permission denied
    await expect(kx.getContent(run.resultRef as string, bogus)).rejects.toBeInstanceOf(
      KxPermissionDenied,
    );
    kx.close();
  });

  it("wait timeout is resumable", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const run = (await kx.invoke(ECHO_HANDLE, { topic: "timeout" })) as Run;
    // Wait on a Mote id that will never appear in this run → times out fast.
    const err = await kx
      ._awaitTerminal(run.instanceIdBytes, new Uint8Array(32), 600, "poll")
      .then(() => null)
      .catch((e: unknown) => e);
    kx.close();
    expect(err).toBeInstanceOf(KxWaitTimeout);
    expect((err as KxWaitTimeout).instanceId).toBe(run.instanceId);
  });

  it("unauthenticated without a token", async () => {
    const s = await authServer();
    const kx = new KxClient(s.endpoint); // no token
    await expect(kx.invoke(ECHO_HANDLE, { topic: "x" }, { wait: true })).rejects.toBeInstanceOf(
      KxUnauthenticated,
    );
    kx.close();
  });

  it("authenticated with a token", async () => {
    const s = await authServer();
    const kx = new KxClient(s.endpoint, { token: s.token });
    const result = (await kx.invoke(ECHO_HANDLE, { topic: "authed" }, { wait: true })) as Result;
    kx.close();
    expect(result.ok).toBe(true);
  });

  it("signatures list empty + unknown not found + bad manifest invalid", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    expect(await kx.listSignatures()).toEqual([]);
    await expect(kx.getSignature("00".repeat(32))).rejects.toBeInstanceOf(KxNotFound);
    await expect(
      kx.registerSignature(new TextEncoder().encode("not a valid signature manifest")),
    ).rejects.toBeInstanceOf(KxInvalidArgument);
    kx.close();
  });

  it("low-level submitRun surfaces server validation (short fingerprint → invalid)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    await expect(
      kx.submitRun({ recipeFingerprint: new TextEncoder().encode("short") }),
    ).rejects.toBeInstanceOf(KxInvalidArgument);
    kx.close();
  });

  it("over-cap args are fail-closed (echo `topic` max is 4096) → invalid_argument", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const topic = "z".repeat(200_000); // far over the recipe's declared bound
    await expect(kx.invoke(ECHO_HANDLE, { topic }, { wait: true })).rejects.toBeInstanceOf(
      KxInvalidArgument,
    );
    kx.close();
  });

  it("a near-max payload round-trips through getContent", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const topic = "z".repeat(4000); // just under the 4096 cap — a non-trivial payload
    const result = (await kx.invoke(ECHO_HANDLE, { topic }, { wait: true })) as Result;
    expect(result.ok).toBe(true);
    expect(result.bytes).not.toBeNull();
    expect((result.bytes as Uint8Array).length).toBeGreaterThan(0);
    const fetched = await kx.getContent(result.resultRef as string, result.instanceId);
    kx.close();
    expect(fetched).toEqual(result.bytes);
  });

  it("concurrent clients: shared instance, isolated per-input terminals", async () => {
    const s = await devServer();
    const N = 6;
    const clients = Array.from({ length: N }, () => new KxClient(s.endpoint));
    const results = (await Promise.all(
      clients.map((kx, i) => kx.invoke(ECHO_HANDLE, { topic: `c${i}` }, { wait: true })),
    )) as Result[];
    for (const kx of clients) kx.close();
    expect(results.every((r) => r.ok)).toBe(true);
    expect(new Set(results.map((r) => r.instanceId)).size).toBe(1); // one run per recipe
    expect(new Set(results.map((r) => r.terminalMoteId)).size).toBe(N); // distinct per input
  });
});
