/** T3.7 Datasets data-plane views — pure, no server. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { DatasetHit, DatasetSummary, IngestResult } from "../src/datasets.js";
import {
  DatasetHitSchema,
  DatasetSummarySchema,
  IngestDocumentsResponseSchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";

const fill = (v: number, n: number): Uint8Array => new Uint8Array(n).fill(v);

describe("DatasetSummary.fromProto", () => {
  it("carries the counts + a snake_case toJSON", () => {
    const d = create(DatasetSummarySchema, {
      datasetId: "corpus",
      name: "corpus",
      docCount: 42n,
      dim: 64,
      createdMs: 1234n,
    });
    const s = DatasetSummary.fromProto(d);
    expect(s.datasetId).toBe("corpus");
    expect(s.docCount).toBe(42);
    expect(s.dim).toBe(64);
    expect(s.createdMs).toBe(1234);
    expect(s.toJSON()).toEqual({
      dataset_id: "corpus",
      name: "corpus",
      doc_count: 42,
      dim: 64,
      created_ms: 1234,
    });
  });
});

describe("DatasetHit.fromProto", () => {
  it("hex-encodes the ref, keeps the bytes + the display-only score, decodes text", () => {
    const h = create(DatasetHitSchema, {
      contentRef: fill(0xab, 32),
      content: new TextEncoder().encode("hello world"),
      score: 0.875,
    });
    const hit = DatasetHit.fromProto(h);
    expect(hit.contentRef).toBe("ab".repeat(32));
    expect(hit.score).toBeCloseTo(0.875);
    expect(hit.text).toBe("hello world");
  });
});

describe("IngestResult.fromProto", () => {
  it("carries the post-ingest counts", () => {
    const r = create(IngestDocumentsResponseSchema, {
      datasetId: "corpus",
      docCount: 10n,
      inserted: 3n,
      dim: 64,
    });
    const res = IngestResult.fromProto(r);
    expect(res.datasetId).toBe("corpus");
    expect(res.docCount).toBe(10);
    expect(res.inserted).toBe(3);
    expect(res.dim).toBe(64);
  });
});
