import type { ScaffoldStatus } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { type ScaffoldRow, deriveScaffoldStatus } from "../../src/lib/scaffold-status";

function stateOf(rows: readonly ScaffoldRow[], path: string): string | undefined {
  return rows.find((r) => r.path === path)?.state;
}

// A dynamic, model-planned project file set (NOT the fixed skeleton) — the server
// is the truth for which files exist (POC-6).
const PROJECT = [
  "package.json",
  "index.html",
  "src/main.tsx",
  "src/App.tsx",
  "src/App.css",
] as const;

function status(over: Partial<ScaffoldStatus>): ScaffoldStatus {
  return {
    phase: "planning",
    filesDone: [],
    filesPending: [],
    detail: "",
    ...over,
  };
}

describe("deriveScaffoldStatus (POC-6, dynamic — driven by real server facts)", () => {
  it("planning: the dynamic set is all pending, none faked done/writing", () => {
    const d = deriveScaffoldStatus(status({ phase: "planning", filesPending: [...PROJECT] }));
    expect(d.rows).toHaveLength(PROJECT.length);
    expect(d.rows.every((r) => r.state === "pending")).toBe(true);
    expect(d.active).toBe(true);
    expect(d.complete).toBe(false);
    expect(d.failed).toBe(false);
    expect(d.phase).toBe("planning");
  });

  it("writing: uses the SERVER's writingPath + surfaces its stream ids", () => {
    const d = deriveScaffoldStatus(
      status({
        phase: "writing",
        filesDone: ["package.json", "index.html"],
        filesPending: ["src/main.tsx", "src/App.tsx", "src/App.css"],
        detail: "writing src/App.tsx",
        writingPath: "src/App.tsx",
        writingInstanceId: "aa".repeat(16),
        writingMoteId: "bb".repeat(32),
      }),
    );
    expect(stateOf(d.rows, "package.json")).toBe("done");
    expect(stateOf(d.rows, "index.html")).toBe("done");
    // The server names the in-flight file — NOT the first pending one.
    expect(stateOf(d.rows, "src/App.tsx")).toBe("writing");
    expect(stateOf(d.rows, "src/main.tsx")).toBe("pending");
    expect(stateOf(d.rows, "src/App.css")).toBe("pending");
    expect(d.rows.filter((r) => r.state === "writing")).toHaveLength(1);
    // The stream ids ride through so the editor can subscribe.
    expect(d.writingPath).toBe("src/App.tsx");
    expect(d.writingInstanceId).toBe("aa".repeat(16));
    expect(d.writingMoteId).toBe("bb".repeat(32));
    expect(d.active).toBe(true);
  });

  it("writing (older server, no writingPath): the first not-done pending is writing", () => {
    const d = deriveScaffoldStatus(
      status({
        phase: "writing",
        filesDone: ["package.json"],
        filesPending: ["index.html", "src/main.tsx"],
      }),
    );
    expect(stateOf(d.rows, "index.html")).toBe("writing");
    expect(stateOf(d.rows, "src/main.tsx")).toBe("pending");
    expect(d.rows.filter((r) => r.state === "writing")).toHaveLength(1);
    // No ids surfaced when the server didn't name a writing file's stream.
    expect(d.writingInstanceId).toBeUndefined();
  });

  it("done: every planned row is done, complete + not active", () => {
    const d = deriveScaffoldStatus(status({ phase: "done", filesDone: [...PROJECT] }));
    expect(d.rows).toHaveLength(PROJECT.length);
    expect(d.rows.every((r) => r.state === "done")).toBe(true);
    expect(d.complete).toBe(true);
    expect(d.active).toBe(false);
    expect(d.failed).toBe(false);
  });

  it("failed: partial files stay visible, the rest pending (no fabricated done)", () => {
    const d = deriveScaffoldStatus(
      status({
        phase: "failed",
        filesDone: ["package.json"],
        filesPending: ["index.html", "src/main.tsx"],
        detail: "the model returned an empty body",
      }),
    );
    expect(stateOf(d.rows, "package.json")).toBe("done");
    expect(d.rows.filter((r) => r.state === "writing")).toHaveLength(0);
    expect(stateOf(d.rows, "index.html")).toBe("pending");
    expect(d.failed).toBe(true);
    expect(d.active).toBe(false);
    expect(d.complete).toBe(false);
  });

  it("the file set is the SERVER's, not a fixed skeleton (dedupes done+pending)", () => {
    const d = deriveScaffoldStatus(
      status({
        phase: "writing",
        filesDone: ["Dockerfile", "package.json"],
        filesPending: ["package.json", "src/index.ts"],
        writingPath: "src/index.ts",
      }),
    );
    // Deduped, done-first — a path in both lists appears once (as done).
    expect(d.rows.map((r) => r.path)).toEqual(["Dockerfile", "package.json", "src/index.ts"]);
    expect(stateOf(d.rows, "package.json")).toBe("done");
    expect(stateOf(d.rows, "src/index.ts")).toBe("writing");
  });
});
