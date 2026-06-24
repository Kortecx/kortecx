/**
 * POC-5d: build a hierarchical file tree from a flat branch manifest (the App's
 * `{path → contentRef}` items). Each "/"-separated path becomes a nested node
 * tree; folders sort before files, alphabetically within each level. A leaf
 * node carries its `contentRef` (the content-store ref to fetch); a folder has
 * `children` and no `contentRef`.
 *
 * Pure + total (no React, no I/O) so the layout is unit-tested directly. The
 * tree is rebuilt from the manifest on every render — manifests are small
 * (a fixed skeleton + a few edits), so an O(n·depth) build is well within budget.
 */

export interface BranchManifestItem {
  readonly path: string;
  readonly contentRef: string;
}

export interface TreeNode {
  /** The path segment (the display name at this level). */
  readonly name: string;
  /** The full path from the root (a leaf's fetch key / a folder's id). */
  readonly path: string;
  /** Present on FILE leaves only — the content-store ref. */
  readonly contentRef?: string;
  /** Present on FOLDER nodes only — the sorted child nodes. */
  readonly children?: TreeNode[];
}

interface MutableNode {
  name: string;
  path: string;
  contentRef?: string;
  children?: Map<string, MutableNode>;
}

/** Folders (have children) sort before files; alphabetical within each kind. */
function compareNodes(a: TreeNode, b: TreeNode): number {
  const aFolder = a.children !== undefined;
  const bFolder = b.children !== undefined;
  if (aFolder !== bFolder) {
    return aFolder ? -1 : 1;
  }
  return a.name.localeCompare(b.name);
}

function freeze(node: MutableNode): TreeNode {
  if (node.children === undefined) {
    // A file leaf — carry its content ref (may be "" defensively; still a leaf).
    return node.contentRef !== undefined
      ? { name: node.name, path: node.path, contentRef: node.contentRef }
      : { name: node.name, path: node.path };
  }
  const children = [...node.children.values()].map(freeze).sort(compareNodes);
  return { name: node.name, path: node.path, children };
}

export function buildFileTree(items: readonly BranchManifestItem[]): TreeNode[] {
  const roots = new Map<string, MutableNode>();

  for (const item of items) {
    // Normalize: split on "/", drop empty segments ("a//b", leading/trailing "/").
    const segments = item.path.split("/").filter((s) => s.length > 0);
    if (segments.length === 0) {
      continue;
    }
    let level = roots;
    let prefix = "";
    for (let i = 0; i < segments.length; i++) {
      const name = segments[i] as string;
      prefix = prefix === "" ? name : `${prefix}/${name}`;
      const isLeaf = i === segments.length - 1;
      let node = level.get(name);
      if (node === undefined) {
        node = { name, path: prefix };
        level.set(name, node);
      }
      if (isLeaf) {
        node.contentRef = item.contentRef;
      } else {
        if (node.children === undefined) {
          node.children = new Map();
        }
        level = node.children;
      }
    }
  }

  return [...roots.values()].map(freeze).sort(compareNodes);
}
