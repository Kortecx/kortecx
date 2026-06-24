import { describe, expect, it } from "vitest";
import { type TreeNode, buildFileTree } from "../../src/lib/file-tree";

const REF = "ab".repeat(32);

function names(nodes: readonly TreeNode[]): string[] {
  return nodes.map((n) => n.name);
}

describe("buildFileTree (POC-5d App project tree)", () => {
  it("nests '/'-separated paths into a hierarchical tree", () => {
    const tree = buildFileTree([
      { path: "README.md", contentRef: REF },
      { path: "prompts/system.md", contentRef: REF },
      { path: "rules/guardrails.md", contentRef: REF },
    ]);
    // Folders (prompts, rules) sort before the README.md file.
    expect(names(tree)).toEqual(["prompts", "rules", "README.md"]);
    const prompts = tree.find((n) => n.name === "prompts");
    expect(prompts?.children).toBeDefined();
    expect(prompts?.contentRef).toBeUndefined();
    expect(prompts?.children?.[0]).toMatchObject({
      name: "system.md",
      path: "prompts/system.md",
      contentRef: REF,
    });
    // The file leaf carries its content ref and no children.
    const readme = tree.find((n) => n.name === "README.md");
    expect(readme?.contentRef).toBe(REF);
    expect(readme?.children).toBeUndefined();
  });

  it("sorts folders before files, alphabetically within each kind", () => {
    const tree = buildFileTree([
      { path: "zeta.md", contentRef: REF },
      { path: "alpha.md", contentRef: REF },
      { path: "src/main.rs", contentRef: REF },
      { path: "docs/intro.md", contentRef: REF },
    ]);
    // docs, src (folders, alpha) then alpha.md, zeta.md (files, alpha).
    expect(names(tree)).toEqual(["docs", "src", "alpha.md", "zeta.md"]);
  });

  it("merges files that share a folder prefix", () => {
    const tree = buildFileTree([
      { path: "a/b/one.md", contentRef: REF },
      { path: "a/b/two.md", contentRef: REF },
      { path: "a/c.md", contentRef: REF },
    ]);
    const a = tree.find((n) => n.name === "a");
    expect(names(a?.children ?? [])).toEqual(["b", "c.md"]);
    const b = a?.children?.find((n) => n.name === "b");
    expect(names(b?.children ?? [])).toEqual(["one.md", "two.md"]);
    expect(b?.path).toBe("a/b");
  });

  it("builds the full path on every node", () => {
    const tree = buildFileTree([{ path: "rules/guardrails.md", contentRef: REF }]);
    const rules = tree.find((n) => n.name === "rules");
    expect(rules?.path).toBe("rules");
    expect(rules?.children?.[0]?.path).toBe("rules/guardrails.md");
  });

  it("ignores empty segments and empty manifests", () => {
    expect(buildFileTree([])).toEqual([]);
    const tree = buildFileTree([
      { path: "/leading.md", contentRef: REF },
      { path: "a//b.md", contentRef: REF },
      { path: "", contentRef: REF },
    ]);
    expect(names(tree)).toEqual(["a", "leading.md"]);
    const a = tree.find((n) => n.name === "a");
    expect(a?.children?.[0]?.path).toBe("a/b.md");
  });
});
