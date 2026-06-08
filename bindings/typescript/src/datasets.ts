/**
 * The Datasets data-plane (RAG) views — a dataset summary, a retrieval hit, and an
 * ingest outcome, as surfaced by `ListDatasets` / `QueryDataset` / `IngestDocuments`
 * (T3.7). Kept in its own module so `types.ts` stays a thin aggregator, mirroring
 * the Rust core's module-per-concern discipline.
 *
 * SN-8: a hit's `score` is DISPLAY-ONLY — never an identity input. The retrieval
 * result a downstream consumer trusts is the ordered `contentRef` SET, matched by
 * EXACT hash. Embedding is pluggable: pass a client-computed `embedding` (the
 * FFI-free path, e.g. via HuggingFace transformers in your app) or omit it to let a
 * gateway with the `inference` feature embed the text server-side.
 */

import type {
  DatasetHit as PbDatasetHit,
  DatasetSummary as PbDatasetSummary,
  IngestDocumentsResponse as PbIngestDocumentsResponse,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One dataset in a `ListDatasets` enumeration. */
export class DatasetSummary {
  constructor(
    readonly datasetId: string,
    readonly name: string,
    readonly docCount: number,
    readonly dim: number,
    /** Unix-ms create time (display only; off every hash). */
    readonly createdMs: number,
  ) {}

  static fromProto(d: PbDatasetSummary): DatasetSummary {
    return new DatasetSummary(d.datasetId, d.name, Number(d.docCount), d.dim, Number(d.createdMs));
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      dataset_id: this.datasetId,
      name: this.name,
      doc_count: this.docCount,
      dim: this.dim,
      created_ms: this.createdMs,
    };
  }
}

/** One retrieval hit: the content-addressed ref (hex), the document bytes, and the
 *  DISPLAY-ONLY similarity score (SN-8 — never an identity input). */
export class DatasetHit {
  constructor(
    readonly contentRef: string,
    readonly content: Uint8Array,
    readonly score: number,
  ) {}

  static fromProto(h: PbDatasetHit): DatasetHit {
    return new DatasetHit(encode(h.contentRef), h.content, h.score);
  }

  /** The retrieved document bytes decoded as UTF-8 (best-effort) — for text corpora. */
  get text(): string {
    return new TextDecoder().decode(this.content);
  }
}

/** The outcome of an `IngestDocuments` call (server-derived counts). */
export class IngestResult {
  constructor(
    readonly datasetId: string,
    readonly docCount: number,
    /** New distinct docs added by this call (post content-addressed dedup). */
    readonly inserted: number,
    readonly dim: number,
  ) {}

  static fromProto(r: PbIngestDocumentsResponse): IngestResult {
    return new IngestResult(r.datasetId, Number(r.docCount), Number(r.inserted), r.dim);
  }
}

/**
 * One document to ingest. `content` is the retrievable payload (always). An
 * OPTIONAL `embedding` takes the FFI-free client-vector path; omit it to let a
 * server embedder (the `inference` feature) embed `content`.
 *
 * `docId` and `metadata` are RESERVED (forward-compat): accepted on the wire but
 * NOT YET persisted or returned. The durable id is always the server-derived
 * content hash (SN-8), so `docId` is advisory; per-doc `metadata` is a planned add.
 */
export interface IngestDoc {
  readonly content: Uint8Array;
  readonly embedding?: readonly number[];
  readonly docId?: string;
  readonly metadata?: Readonly<Record<string, string>>;
}
