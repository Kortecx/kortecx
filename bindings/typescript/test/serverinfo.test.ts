/** POC-1 Settings server-info view — pure, no server. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { GetServerInfoResponseSchema } from "../src/gen/kortecx/v1/gateway_pb.js";
import { ServerInfo } from "../src/serverinfo.js";

describe("ServerInfo.fromProto", () => {
  it("projects the config + converts uint64 caps to number + a snake_case toJSON", () => {
    const r = create(GetServerInfoResponseSchema, {
      modelId: "kx-serve:gemma-4-12b",
      modelPath: "/models/gemma-4-12b.gguf",
      listenAddr: "127.0.0.1:50151",
      wsAddr: "127.0.0.1:50152",
      consoleAddr: "127.0.0.1:50150",
      metricsAddr: "",
      contentRoot: "/var/kx/content",
      journalPath: "/var/kx/journal.db",
      catalogDir: "/var/kx/catalog",
      maxLease: 16n,
      contentMaxBytes: 33_554_432n,
      corsOrigins: ["https://app.example.com"],
      tlsEnabled: true,
      authMode: "token",
      featureHnsw: true,
      featureInference: true,
      featureConsole: true,
      featureVision: false,
      auditLogEnabled: true,
    });
    const info = ServerInfo.fromProto(r);
    expect(info.modelId).toBe("kx-serve:gemma-4-12b");
    expect(info.listenAddr).toBe("127.0.0.1:50151");
    // uint64 caps land as plain numbers (the SDK convention for non-id uint64s).
    expect(info.maxLease).toBe(16);
    expect(typeof info.maxLease).toBe("number");
    expect(info.contentMaxBytes).toBe(33_554_432);
    expect(typeof info.contentMaxBytes).toBe("number");
    expect(info.corsOrigins).toEqual(["https://app.example.com"]);
    expect(info.tlsEnabled).toBe(true);
    expect(info.authMode).toBe("token");
    expect(info.featureVision).toBe(false);
    expect(info.toJSON()).toEqual({
      model_id: "kx-serve:gemma-4-12b",
      model_path: "/models/gemma-4-12b.gguf",
      listen_addr: "127.0.0.1:50151",
      ws_addr: "127.0.0.1:50152",
      console_addr: "127.0.0.1:50150",
      metrics_addr: "",
      content_root: "/var/kx/content",
      journal_path: "/var/kx/journal.db",
      catalog_dir: "/var/kx/catalog",
      max_lease: 16,
      content_max_bytes: 33_554_432,
      cors_origins: ["https://app.example.com"],
      tls_enabled: true,
      auth_mode: "token",
      feature_hnsw: true,
      feature_inference: true,
      feature_console: true,
      feature_vision: false,
      audit_log_enabled: true,
    });
  });

  it("carries the honest empty model-less / FFI-free serve shape", () => {
    const r = create(GetServerInfoResponseSchema, {
      modelId: "",
      modelPath: "",
      consoleAddr: "",
      metricsAddr: "",
      maxLease: 0n,
      contentMaxBytes: 0n,
      corsOrigins: [],
      tlsEnabled: false,
      authMode: "deny-all",
      featureInference: false,
    });
    const info = ServerInfo.fromProto(r);
    expect(info.modelId).toBe("");
    expect(info.consoleAddr).toBe("");
    expect(info.corsOrigins).toEqual([]);
    expect(info.maxLease).toBe(0);
    expect(info.featureInference).toBe(false);
    expect(info.authMode).toBe("deny-all");
  });
});
