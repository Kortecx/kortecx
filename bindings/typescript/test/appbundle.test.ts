/**
 * `kortecx.appbundle/v1` codec — cross-surface golden parity + structure tests, plus
 * a server-backed export → import → clone round-trip.
 *
 * The parity gate (GR12): every committed bundle in `tests/golden/apps/bundle_corpus.json`
 * round-trips through THIS SDK's codec BYTE-IDENTICALLY (matches the Rust `kx-appbundle`
 * crate + the Python SDK). `contentRefs` mirrors the Rust envelope walk.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { afterEach, describe, expect, it } from "vitest";

import { AppBundle, BUNDLE_SCHEMA, KxClient, app, contentRefs, flow } from "../src/node.js";
import { devServer, stopAllServers } from "./fixtures/serve.js";

const BUNDLE_CORPUS = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "..",
  "tests",
  "golden",
  "apps",
  "bundle_corpus.json",
);
const cases: { name: string; bundle: string }[] = JSON.parse(readFileSync(BUNDLE_CORPUS, "utf-8"));

afterEach(async () => {
  await stopAllServers();
});

describe("appbundle codec (no server)", () => {
  it.each(cases)("round-trips $name byte-identically", ({ bundle }) => {
    expect(AppBundle.fromJson(bundle).toJson()).toBe(bundle);
  });

  it("rejects a bad schema", () => {
    const wire = JSON.stringify({ app_digest: "a".repeat(64), envelope: "{}", schema: "x" });
    expect(() => AppBundle.fromJson(wire)).toThrow();
  });

  it("rejects a bad hex ref", () => {
    const wire = JSON.stringify({ app_digest: "NOPE", envelope: "{}", schema: BUNDLE_SCHEMA });
    expect(() => AppBundle.fromJson(wire)).toThrow();
  });

  it("omits blobs + source_digest when empty", () => {
    const wire = new AppBundle("ab".repeat(32), new TextEncoder().encode("{}")).toJson();
    expect(wire).not.toContain("blobs");
    expect(wire).not.toContain("source_digest");
    const parsed = AppBundle.fromJson(wire);
    expect(parsed.appDigest).toBe("ab".repeat(32));
    expect(parsed.blobCount()).toBe(0);
  });

  it("round-trips a binary blob + lineage", () => {
    const blobs = new Map([["aa".repeat(32), new Uint8Array([0, 1, 2, 253, 254, 255])]]);
    const b = new AppBundle(
      "11".repeat(32),
      new TextEncoder().encode('{"name":"x"}'),
      blobs,
      "22".repeat(32),
    );
    const parsed = AppBundle.fromJson(b.toJson());
    expect(parsed.toJson()).toBe(b.toJson());
    expect([...(parsed.blobs.get("aa".repeat(32)) ?? [])]).toEqual([0, 1, 2, 253, 254, 255]);
    expect(parsed.sourceDigest).toBe("22".repeat(32));
  });

  it("contentRefs walks, sorts, and gates datasets", () => {
    const envelope = {
      references: {
        prompts: [{ name: "p", content_ref: "aa".repeat(32) }],
        rules: [{ name: "r", content_ref: "bb".repeat(32) }],
        skills: [{ name: "s", instructions_ref: "cc".repeat(32) }],
        datasets: [{ dataset_ref: "d", cas_refs: ["dd".repeat(32)] }],
      },
      steering_config: { context: { context_refs: ["ee".repeat(32)] } },
    };
    expect(contentRefs(envelope)).toEqual([
      "aa".repeat(32),
      "bb".repeat(32),
      "cc".repeat(32),
      "ee".repeat(32),
    ]);
    const withData = contentRefs(envelope, true);
    expect(withData).toContain("dd".repeat(32));
    expect(withData.length).toBe(5);
  });
});

describe("appbundle round-trip (real kx serve)", () => {
  it("export -> import -> clone lands the same app_digest + lineage", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    const a = app("Bundle Demo TS")
      .blueprint(flow().step({ topic: "hi" }))
      .rule("cite", { body: "Always cite your sources." });
    const saved = await a.save({ client: kx });
    const handle = saved.handle;
    const original = await kx.getApp(handle);
    expect(original).not.toBeNull();
    const originalDigest = original?.appDigest ?? "";
    expect(originalDigest.length).toBe(64);
    expect(original?.sourceDigest).toBe(""); // authored here

    // EXPORT: a bundle carrying the rule blob, named by the App's app_digest.
    const wire = await kx.exportAppBundle(handle);
    const bundle = AppBundle.fromJson(wire);
    expect(bundle.appDigest).toBe(originalDigest);
    expect(bundle.blobCount()).toBe(1);

    // IMPORT (force, same handle): app_digest round-trips + lineage stamped.
    await kx.importApp(wire, { force: true });
    const reimported = await kx.getApp(handle);
    expect(reimported?.appDigest).toBe(originalDigest);
    expect(reimported?.sourceDigest).toBe(originalDigest);

    // CLONE: a new App under a new name, with lineage back to the source.
    const cloned = await kx.cloneApp(handle, "Bundle Copy TS");
    expect(cloned.handle).toBe("apps/local/bundle-copy-ts");
    const copy = await kx.getApp("apps/local/bundle-copy-ts");
    expect((copy?.envelope as Record<string, unknown>).name).toBe("Bundle Copy TS");
    expect(copy?.sourceDigest).toBe(originalDigest);
    expect(copy?.appDigest).not.toBe(originalDigest); // rename ⇒ different digest
  });
});
