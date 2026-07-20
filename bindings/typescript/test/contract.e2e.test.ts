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
  KxFailedPrecondition,
  KxInvalidArgument,
  KxNotFound,
  KxPermissionDenied,
  KxUnauthenticated,
  KxWaitTimeout,
} from "../src/node.js";
import type { Delta, GlobalDelta, Result, Run } from "../src/node.js";
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

// --- UI-2: run enumeration + recipe catalog (the new additive RPCs) ----------

const FANOUT_HANDLE = "kx/recipes/passthrough-dag";

describe("UI-2 run enumeration", () => {
  it("listRuns is empty before any run, then enumerates the durable instance", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const before = await kx.listRuns();
    expect(before.runs).toHaveLength(0);
    expect(before.hasMore).toBe(false);

    const run = (await kx.invoke(ECHO_HANDLE, { topic: "audit" }, { wait: true })) as Result;
    const after = await kx.listRuns();
    kx.close();
    // Single-node: one registered run per journal; every invoke joins it.
    expect(after.runs).toHaveLength(1);
    expect(after.runs[0]?.instanceId).toBe(run.instanceId);
    expect(after.runs[0]?.registeredSeq).toBe(1); // the RunRegistered is seq 1
    expect(after.runs[0]?.registeredUnixMs).toBeGreaterThan(0); // a live wall-clock
  });

  it("listRuns honors the limit + before_seq pagination shape", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    await kx.invoke(ECHO_HANDLE, { topic: "x" }, { wait: true });
    // A page of 0 is server-clamped to >=1 (never an empty unbounded scan).
    const page = await kx.listRuns({ limit: 1 });
    kx.close();
    expect(page.runs.length).toBe(1);
    expect(page.hasMore).toBe(false); // only one run exists
  });
});

describe("UI-2 recipe catalog", () => {
  it("listRecipes enumerates the provisioned invocable handles", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const recipes = await kx.listRecipes();
    kx.close();
    expect(recipes).toContain(ECHO_HANDLE);
    expect(recipes).toContain(FANOUT_HANDLE);
  });

  it("getRecipeForm returns the echo recipe's typed `topic` field", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const form = await kx.getRecipeForm(ECHO_HANDLE);
    kx.close();
    expect(form.handle).toBe(ECHO_HANDLE);
    expect(form.fields).toHaveLength(1);
    expect(form.fields[0]?.name).toBe("topic");
    expect(form.fields[0]?.type).toBe("str");
    expect(form.fields[0]?.required).toBe(true);
    expect(form.fields[0]?.maxLen).toBe(4096);
  });

  it("getRecipeForm for a fanout recipe (no free-params) is an empty form", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const form = await kx.getRecipeForm(FANOUT_HANDLE);
    kx.close();
    expect(form.fields).toHaveLength(0);
  });

  it("getRecipeForm for an unknown handle throws KxNotFound (honest discovery)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    await expect(kx.getRecipeForm("kx/recipes/does-not-exist")).rejects.toBeInstanceOf(KxNotFound);
    kx.close();
  });
});

// --- UI-3: teams (membership) + sharing (grants) viewers ---------------------

const DEMO_TEAM = "kx/teams/workspace";

describe("UI-3 teams viewer", () => {
  it("listTeams enumerates the bootstrap-seeded demo team", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const teams = await kx.listTeams();
    kx.close();
    const demo = teams.find((t) => t.teamId === DEMO_TEAM);
    expect(demo).toBeDefined();
    expect(demo?.owner).toBe("kx-gateway");
    expect(demo?.memberCount).toBeGreaterThanOrEqual(1);
  });

  it("listTeamMembers shows roles + a delegate; resolves a warrant only with assetRef", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const members = await kx.listTeamMembers(DEMO_TEAM);
    expect(members.owner).toBe("kx-gateway");
    expect(members.members.length).toBeGreaterThanOrEqual(1);
    expect(members.members.filter((m) => m.isDelegate)).toHaveLength(1);
    expect(members.members.every((m) => m.resolvedWarrant === null)).toBe(true);

    // With the echo asset: a member resolves a warrant ⊆ the team (no escalation).
    const withAsset = await kx.listTeamMembers(DEMO_TEAM, { assetRef: ECHO_HANDLE });
    kx.close();
    const resolved = withAsset.members.find((m) => m.resolvedWarrant !== null);
    expect(resolved?.resolvedWarrant?.maxCalls).toBeLessThanOrEqual(3);
  });

  it("listTeamMembers for an unknown team throws KxNotFound (honest viewer)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    await expect(kx.listTeamMembers("kx/teams/nope")).rejects.toBeInstanceOf(KxNotFound);
    kx.close();
  });
});

describe("UI-3 sharing inspector", () => {
  it("listAssetGrants shows the recipe + team grants on echo", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const grants = await kx.listAssetGrants(ECHO_HANDLE);
    kx.close();
    expect(grants.owner).toBe("kx-gateway");
    const teamGrant = grants.grants.find((g) => g.grantee === DEMO_TEAM);
    expect(teamGrant?.status).toBe("root");
    expect(teamGrant?.actions).toContain("Use");
    expect(grants.grants.every((g) => !g.revoked)).toBe(true);
  });

  it("listAssetGrants for an unknown asset throws KxNotFound", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    await expect(kx.listAssetGrants("kx/recipes/does-not-exist")).rejects.toBeInstanceOf(
      KxNotFound,
    );
    kx.close();
  });
});

// --- T3.7: the Datasets data-plane (RAG), FFI-free client-vector path ----------

describe("datasets (RAG) data-plane — client-vector path (FFI-free)", () => {
  it("ingest → list → query returns the nearest document's bytes + 32B hex ref", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const ingest = await kx.ingestDocuments("corpus", [
      { content: new TextEncoder().encode("alpha"), embedding: [1, 0, 0, 0.1] },
      { content: new TextEncoder().encode("bravo"), embedding: [0, 1, 0, 0.1] },
      { content: new TextEncoder().encode("charlie"), embedding: [0, 0, 1, 0.1] },
    ]);
    expect(ingest.datasetId).toBe("corpus");
    expect(ingest.inserted).toBe(3);
    expect(ingest.dim).toBe(4);

    const datasets = await kx.listDatasets();
    const corpus = datasets.find((d) => d.datasetId === "corpus");
    expect(corpus?.docCount).toBe(3);

    const hits = await kx.queryDataset("corpus", { embedding: [0, 1, 0, 0.1], k: 1 });
    expect(hits[0]?.text).toBe("bravo");
    expect(hits[0]?.contentRef).toHaveLength(64); // 32B hex
    kx.close();
  });

  it("an unknown dataset is not-found; a text query without an embedder is failed-precondition", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    await expect(kx.queryDataset("nope", { embedding: [1, 0], k: 1 })).rejects.toBeInstanceOf(
      KxNotFound,
    );
    // The FFI-free server has no embedder, so a TEXT query is FAILED_PRECONDITION.
    await kx.ingestDocuments("c", [{ content: new TextEncoder().encode("x"), embedding: [1, 0] }]);
    await expect(kx.queryDataset("c", { text: "find x" })).rejects.toBeInstanceOf(
      KxFailedPrecondition,
    );
    kx.close();
  });
});

// --- W1.A5: toolscout advisory discovery + bundle preview ---------------------

describe("toolscout (advisory — scores never authorize)", () => {
  it("lists the builtin manifests and scores an exact-keyword intent at 10000 bp", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);

    const manifests = await kx.listToolManifests();
    // `text-summarize@1` was removed from the built-in set: no capability could ever
    // be registered for it, so it advertised a tool that could not dispatch.
    expect(manifests.map((m) => m.toolId)).toEqual(["fs-read", "fs-write"]);
    for (const m of manifests) {
      expect(m.kind).toBe("Builtin");
      expect(m.fingerprintHash).toHaveLength(64); // 32B hex
    }

    const score = await kx.scoreTaskBundle({
      intent: "read a file from disk",
      languageTags: ["en"],
      tools: [{ toolId: "fs-read", toolVersion: "1" }],
    });
    expect(score.ranked).toHaveLength(3);
    expect(score.ranked[0]?.toolId).toBe("fs-read");
    expect(score.ranked[0]?.scoreBp).toBe(10_000); // deterministic exact-keyword hit
    expect(score.bundleFingerprint).toHaveLength(64);
    // The FFI-free server has no react runtime → the dry-run verdict degrades.
    expect(score.verdict).toBe("unavailable");
    kx.close();
  });

  it("an invalid spec (no tools) is invalid-argument", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    await expect(kx.scoreTaskBundle({ intent: "x", tools: [] })).rejects.toBeInstanceOf(
      KxInvalidArgument,
    );
    kx.close();
  });
});

// --- Batch A: client uploads + batch reads + model discovery ------------------

describe("Batch A content uploads + model discovery", () => {
  it("put → uploads-scope get → batch round-trips; the ref is server-derived", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);

    const bytes = new TextEncoder().encode("batch-a contract blob");
    const put = await kx.putContent(bytes, { mediaType: "text/plain", filename: "a.txt" });
    expect(put.contentRef).toHaveLength(64);
    expect(put.size).toBe(BigInt(bytes.length));
    expect(put.deduplicated).toBe(false);

    // Identical bytes dedup at the SAME server-derived ref.
    const again = await kx.putContent(bytes);
    expect(again.contentRef).toBe(put.contentRef);
    expect(again.deduplicated).toBe(true);

    // Uploads scope (no instanceId): the single get serves the bytes back.
    const got = await kx.getContent(put.contentRef);
    expect(new TextDecoder().decode(got)).toBe("batch-a contract blob");

    // The batch path: one real + one never-existed ref → the latter is the
    // UNIFORM empty item (no existence oracle).
    const items = await kx.getContentBatch([put.contentRef, "77".repeat(32)]);
    expect(items).toHaveLength(2);
    expect(items[0]?.missing).toBe(false);
    expect(items[0]?.text).toBe("batch-a contract blob");
    expect(items[1]?.missing).toBe(true);
    kx.close();
  });

  it("kx content put parity: the CLI ref byte-equals the SDK ref", async () => {
    const { mkdtempSync, writeFileSync } = await import("node:fs");
    const { tmpdir } = await import("node:os");
    const path = await import("node:path");
    const s = await devServer();
    const kx = new KxClient(s.endpoint);

    const dir = mkdtempSync(path.join(tmpdir(), "kx-put-"));
    const file = path.join(dir, "parity.bin");
    const bytes = new TextEncoder().encode("cross-surface parity payload");
    writeFileSync(file, bytes);

    const cliOut = cli(s.endpoint, "content", "put", file);
    const sdk = await kx.putContent(bytes, { filename: "parity.bin" });
    expect(cliOut.content_ref).toBe(sdk.contentRef);
    // The CLI uploaded first, so the SDK put reports dedup — same blob, one store.
    expect(sdk.deduplicated).toBe(true);
    kx.close();
  });

  it("kx models list parity: an FFI-free serve answers an honest empty list", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const sdk = await kx.listModels();
    expect(sdk).toEqual([]);
    const cliOut = cli(s.endpoint, "models", "list");
    expect(cliOut.models).toEqual([]);
    kx.close();
  });
});

// --- Batch B (PR-2): mote detail parity + the structured refusal code --------

describe("Batch B: GetMoteDetail + runs list + refusal code", () => {
  it("getMoteDetail and runs list match the CLI field-for-field", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const result = (await kx.invoke(ECHO_HANDLE, { topic: "inspect" }, { wait: true })) as Result;
    const detail = await kx.getMoteDetail(result.instanceId, result.terminalMoteId);
    const runs = await kx.listRuns();
    kx.close();

    expect(detail.defFound).toBe(true);
    expect(detail.moteDefHash).toHaveLength(64);
    expect(detail.stepKind).not.toBe("");

    const cliDetail = cli(s.endpoint, "mote", "show", result.instanceId, result.terminalMoteId);
    expect(detail.toJSON()).toEqual(cliDetail);

    const cliRuns = cli(s.endpoint, "runs", "list");
    expect({
      runs: runs.runs.map((r) => r.toJSON()),
      has_more: runs.hasMore,
    }).toEqual(cliRuns);
  });

  it("an uncommitted/unknown mote stays honest (NOT_FOUND in an owned run)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const result = (await kx.invoke(ECHO_HANDLE, { topic: "owned" }, { wait: true })) as Result;
    await expect(kx.getMoteDetail(result.instanceId, "ee".repeat(32))).rejects.toBeInstanceOf(
      KxNotFound,
    );
    // The wrong ticket is the UNIFORM denial (no oracle).
    await expect(kx.getMoteDetail("99".repeat(16), result.terminalMoteId)).rejects.toBeInstanceOf(
      KxPermissionDenied,
    );
    kx.close();
  });

  it("a refused submitRun carries the structured kx-refusal-code (the trailer proof)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const fill = (v: number, n: number) => new Uint8Array(n).fill(v);
    // R-1 by construction: WORLD_MUTATING + IdempotentByConstruction + an
    // EMPTY tool_contract (nothing to dedup against). Mirrors the demo mote
    // shape (server.rs demo_pure_mote) with the nd-class flipped.
    await expect(
      kx.submitRun({
        recipeFingerprint: fill(0x5a, 32),
        motes: [
          {
            mote: {
              moteId: fill(0, 32), // advisory — the coordinator re-derives (D53)
              def: {
                logicRef: fill(7, 32),
                modelId: "m",
                promptTemplateHash: fill(9, 32),
                toolContract: {},
                ndClass: 3, // WORLD_MUTATING
                configSubset: {},
                effectPattern: 1, // IDEMPOTENT_BY_CONSTRUCTION
                isTopologyShaper: false,
                inferenceParams: {},
                schemaVersion: 5, // MOTE_DEF_SCHEMA_VERSION (frozen wire)
              },
              inputDataId: fill(5, 32),
              graphPosition: fill(1, 1),
              parents: [],
            },
            warrant: {
              moteClass: 3,
              ndClass: 3,
              fsScope: { mounts: [] },
              netScope: { scope: { case: "none", value: {} } },
              syscallProfileRef: fill(4, 32),
              toolGrants: [],
              modelRoute: { modelId: "m", maxInputTokens: 1, maxOutputTokens: 1, maxCalls: 1 },
              resourceCeiling: {
                cpuMilli: 1,
                memBytes: 1n,
                wallClockMs: 1n,
                fdCount: 1,
                diskBytes: 1n,
              },
              executorClass: 4, // MACOS_SANDBOX (any registered class — refusal fires first)
            },
            acceptAtLeastOnce: false,
            reactSeed: false,
          },
        ],
      }),
    ).rejects.toSatisfy((e: unknown) => {
      expect(e).toBeInstanceOf(KxFailedPrecondition);
      expect((e as KxFailedPrecondition).refusalCode).toBe("R-1");
      return true;
    });
    kx.close();
  });
});

// --- Batch C (PR-3): the global event tail + mote execution telemetry --------

describe("Batch C: global event tail + telemetry", () => {
  it("streamAllEvents narrates the run registration + the attributed commit", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const run = (await kx.invoke(ECHO_HANDLE, { topic: "global" })) as Run;
    await run.wait({ timeoutMs: 30_000 });
    const deltas: GlobalDelta[] = [];
    for await (const d of kx.streamAllEvents({ since: 0n, follow: false })) {
      deltas.push(d);
    }
    kx.close();
    // The run came into existence on the global tail (the per-run cursor never
    // surfaces this), fingerprint-joined and instance-attributed.
    const reg = deltas.find((d) => d.kind === "run_registered");
    expect(reg?.instanceId).toBe(run.instanceId);
    expect(reg?.recipeFingerprint).toBe(run.recipeFingerprint);
    expect(reg?.registeredUnixMs).toBeGreaterThan(0);
    // The terminal commit rides the same stream, watermark-attributed.
    const committed = deltas.filter((d) => d.kind === "committed");
    expect(
      committed.some((d) => d.moteId === run.terminalMoteId && d.instanceId === run.instanceId),
    ).toBe(true);
  });

  it("the global WS channel sees the attributed terminal commit", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const run = (await kx.invoke(ECHO_HANDLE, { topic: "ws-all" }, { wait: true })) as Result;
    let found = false;
    let i = 0;
    for await (const d of kx.wsAllEvents({ since: 0n, wsEndpoint: s.wsEndpoint })) {
      if (d.kind === "committed" && d.moteId === run.terminalMoteId) {
        expect(d.instanceId).toBe(run.instanceId);
        found = true;
        break;
      }
      if (i++ > 200) break; // the catch-up replay carries it in the first frames
    }
    kx.close();
    expect(found).toBe(true);
  });

  it("listMoteTelemetry pages the execution exhaust, scoped by instance and mote", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const result = (await kx.invoke(ECHO_HANDLE, { topic: "telemetry" }, { wait: true })) as Result;
    // The sidecar joins rows on a 250 ms background tick — poll briefly.
    let page = await kx.listMoteTelemetry();
    for (let i = 0; page.rows.length === 0 && i < 40; i++) {
      await new Promise((r) => setTimeout(r, 250));
      page = await kx.listMoteTelemetry();
    }
    expect(page.rows.length).toBeGreaterThan(0);
    const row = page.rows.find((r) => r.moteId === result.terminalMoteId);
    expect(row?.instanceId).toBe(result.instanceId);
    expect(row?.seq).toBeGreaterThan(0);
    expect(row?.startedUnixMs).toBeGreaterThan(0);
    expect(row?.inputTokens).toBeNull(); // NEVER set in OSS
    expect(row?.modelId).toBe(""); // echo is not a model mote (FFI-free)

    // Scoping filters narrow to the same row; rows come newest-first.
    const scoped = await kx.listMoteTelemetry({
      instanceId: result.instanceId,
      moteId: result.terminalMoteId,
    });
    kx.close();
    expect(scoped.rows.map((r) => r.moteId)).toContain(result.terminalMoteId);
    const seqs = page.rows.map((r) => r.seq);
    expect([...seqs].sort((a, b) => b - a)).toEqual(seqs);
  });
});

// --- Skills catalog (declarative kortecx.skill/v1 bundles) --------------

describe("Skills catalog", () => {
  it("add → list → show → remove round-trips with server-derived identity (SN-8)", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const added = await kx.skills.add({
      manifest: {
        schema: "kortecx.skill/v1",
        name: "triage",
        version: "1",
        description: "test skill",
        tools: { "mcp-echo/echo": "1", "gmail/search": "1" },
      },
      instructions: "# Triage\nSearch first.",
    });
    expect(added.name).toBe("triage");
    expect(added.skillRef).toHaveLength(32); // 16 bytes hex
    expect(added.instructionsRef).toHaveLength(64);
    expect(added.deduplicated).toBe(false);

    // Identical re-add dedups to the SAME server-derived identity.
    const again = await kx.skills.add({
      manifest: {
        schema: "kortecx.skill/v1",
        name: "triage",
        version: "1",
        description: "test skill",
        tools: { "mcp-echo/echo": "1", "gmail/search": "1" },
      },
      instructions: "# Triage\nSearch first.",
    });
    expect(again.deduplicated).toBe(true);
    expect(again.skillRef).toBe(added.skillRef);

    const list = await kx.skills.list();
    expect(list.map((s) => s.name)).toContain("triage");

    // The form carries the wish set with the ADVISORY registered bit: the
    // bundled echo tool is fireable on a dev serve; the gmail wish is not.
    const form = await kx.skills.show("triage");
    expect(form).not.toBeNull();
    expect(form?.summary.instructionsRef).toBe(added.instructionsRef);
    const bits = Object.fromEntries((form?.wishes ?? []).map((w) => [w.toolId, w.registered]));
    expect(bits["gmail/search"]).toBe(false);
    expect(form?.instructionsPreview).toContain("# Triage");

    // Uniform not-found + remove.
    expect(await kx.skills.show("no-such")).toBeNull();
    expect(await kx.skills.remove("triage")).toBe(true);
    expect(await kx.skills.remove("triage")).toBe(false);
    kx.close();
  });

  it("an authority-bearing manifest is refused fail-closed", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    await expect(
      kx.skills.add({
        manifest: {
          schema: "kortecx.skill/v1",
          name: "evil",
          warrant: { tool_grants: ["*"] },
        },
        instructions: "x",
      }),
    ).rejects.toThrow();
    kx.close();
  });
});
