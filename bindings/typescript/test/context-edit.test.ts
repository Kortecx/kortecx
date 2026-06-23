/**
 * POC-2 context-edit SDK helpers over a real `kx serve`. The edit family is pure
 * CLIENT composition over existing RPCs (GetContextBundle → PutContent →
 * PutContextBundle re-upsert): no proto/journal change, digest-invariant by
 * construction. Mirrors the Python `test_context_edit.py`: round-trip
 * edit/remove/export, the stale-base optimistic-concurrency guard, the
 * empty-bundle refusal, and the selector error cases (driven through the public
 * methods, since `resolveContextItem` is private).
 */

import { afterEach, describe, expect, it } from "vitest";
import { KxClient, KxFailedPrecondition, KxUsage } from "../src/node.js";
import { devServer, stopAllServers } from "./fixtures/serve.js";

const enc = new TextEncoder();
const dec = new TextDecoder();

afterEach(async () => {
  await stopAllServers();
});

describe("context-edit helpers", () => {
  it("round-trips edit / export / remove and preserves name + media + description", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    try {
      const a = (await kx.putContent(enc.encode("alpha"), { mediaType: "text/plain" })).contentRef;
      const b = (await kx.putContent(enc.encode("beta"), { mediaType: "text/plain" })).contentRef;
      const put = await kx.putContextBundle(
        "t/ctx/docs",
        [
          { name: "a", contentRef: a, mediaType: "text/plain" },
          { name: "b", contentRef: b, mediaType: "text/plain" },
        ],
        { description: "d" },
      );

      expect(dec.decode(await kx.exportContextItem("t/ctx/docs", "a"))).toBe("alpha");
      expect(dec.decode(await kx.exportContextItem("t/ctx/docs", 1))).toBe("beta");

      const res = await kx.editContextItem("t/ctx/docs", "a", enc.encode("ALPHA-v2"));
      expect(res.bundleRef).not.toBe(put.bundleRef);
      expect(dec.decode(await kx.exportContextItem("t/ctx/docs", "a"))).toBe("ALPHA-v2");
      const bundle = await kx.getContextBundle("t/ctx/docs");
      expect(bundle?.description).toBe("d");
      expect(bundle?.items.map((i) => i.name).sort()).toEqual(["a", "b"]);

      // Editing back to identical bytes is a content-layer dedup hit.
      const again = await kx.editContextItem("t/ctx/docs", "a", enc.encode("ALPHA-v2"));
      expect(again.deduplicated).toBe(true);

      await kx.removeContextItem("t/ctx/docs", "b");
      const left = await kx.getContextBundle("t/ctx/docs");
      expect(left?.items.map((i) => i.name)).toEqual(["a"]);
      await expect(kx.exportContextItem("t/ctx/docs", "b")).rejects.toBeInstanceOf(KxUsage);

      // Removing the last item is refused (use deleteContextBundle instead).
      await expect(kx.removeContextItem("t/ctx/docs", "a")).rejects.toThrow(/empty/);
    } finally {
      kx.close();
    }
  });

  it("the stale-base guard fails closed on a concurrent change", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    try {
      const a = (await kx.putContent(enc.encode("v1"), { mediaType: "text/plain" })).contentRef;
      const put = await kx.putContextBundle("t/ctx/race", [
        { name: "a", contentRef: a, mediaType: "text/plain" },
      ]);
      const stale = put.bundleRef;

      // A concurrent writer changes the bundle (a second item ⇒ a new bundleRef).
      const b = (await kx.putContent(enc.encode("added"), { mediaType: "text/plain" })).contentRef;
      await kx.putContextBundle("t/ctx/race", [
        { name: "a", contentRef: a, mediaType: "text/plain" },
        { name: "b", contentRef: b, mediaType: "text/plain" },
      ]);

      await expect(
        kx.editContextItem("t/ctx/race", "a", enc.encode("v2"), { expectBundleRef: stale }),
      ).rejects.toBeInstanceOf(KxFailedPrecondition);

      const current = await kx.getContextBundle("t/ctx/race");
      await kx.editContextItem("t/ctx/race", "a", enc.encode("v2"), {
        expectBundleRef: current?.bundleRef,
      });
      expect(dec.decode(await kx.exportContextItem("t/ctx/race", "a"))).toBe("v2");
    } finally {
      kx.close();
    }
  });

  it("rejects unknown, ambiguous, and out-of-range item selectors", async () => {
    const s = await devServer();
    const kx = new KxClient(s.endpoint);
    try {
      const r = (await kx.putContent(enc.encode("x"), { mediaType: "text/plain" })).contentRef;
      await kx.putContextBundle("t/ctx/sel", [
        { name: "dup", contentRef: r, mediaType: "text/plain" },
        { name: "dup", contentRef: r, mediaType: "text/plain" },
      ]);
      await expect(kx.exportContextItem("t/ctx/sel", "missing")).rejects.toBeInstanceOf(KxUsage);
      await expect(kx.exportContextItem("t/ctx/sel", "dup")).rejects.toThrow(/ambiguous/);
      await expect(kx.exportContextItem("t/ctx/sel", 9)).rejects.toThrow(/out of range/);
      // The index disambiguates a duplicate name.
      expect(dec.decode(await kx.exportContextItem("t/ctx/sel", 1))).toBe("x");
    } finally {
      kx.close();
    }
  });
});
