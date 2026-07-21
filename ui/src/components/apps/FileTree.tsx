/**
 * POC-5d: the App project file tree (left pane of the App detail view). Renders
 * the hierarchical {@link TreeNode} list from {@link buildFileTree}: folders are
 * expand/collapse buttons (aria-expanded), files are selectable buttons. Token-only
 * colours, keyboard-accessible, both themes. Selection is owned by the parent (the
 * selected path is highlighted); the tree itself is presentational.
 *
 * THE FILENAME IS THE ROW. This pane lives in a 180-240px rail, and a row that also
 * carried a content-hash chip gave the hash ~110px of it — `.digestchip` is
 * `flex-shrink: 0` by design (correct in the wide dense rows it was built for), so the
 * NAME absorbed the entire width deficit and truncated to about six characters. Two
 * sibling components both rendered `Counte…`, which is indistinguishable from the
 * project never having been generated. The ref now lives in the file-pane head, where
 * there is width for it; here every row carries its full path on `title=`, so anything
 * that still clips is recoverable on hover. (Dropping the chip also un-nests a `<button>`
 * from inside a `<button>` — invalid HTML, already fixed the same way in ArtifactGallery.)
 */

import { useState } from "react";
import type { FileTreeState, TreeNode } from "../../lib/file-tree";

/** POC-6: the per-file authoring glyph shown in a live creation tree. */
const STATE_GLYPH: Record<FileTreeState, string> = {
  done: "✓",
  writing: "◐",
  pending: "·",
};

/** Words for the authoring glyph, so the state reaches the tooltip and a screen reader
 *  rather than living only in a shape. */
const STATE_LABEL: Record<FileTreeState, string> = {
  done: "written",
  writing: "being written now",
  pending: "not written yet",
};

export function FileTree({
  nodes,
  selectedPath,
  onSelect,
}: {
  nodes: readonly TreeNode[];
  selectedPath: string | null;
  onSelect: (path: string, contentRef: string) => void;
}) {
  if (nodes.length === 0) {
    return (
      <p className="muted file-tree__empty" data-testid="file-tree-empty">
        No files in this App's project yet.
      </p>
    );
  }
  return (
    <ul className="file-tree" data-testid="file-tree">
      {nodes.map((node) => (
        <TreeRow
          key={node.path}
          node={node}
          depth={0}
          selectedPath={selectedPath}
          onSelect={onSelect}
        />
      ))}
    </ul>
  );
}

function TreeRow({
  node,
  depth,
  selectedPath,
  onSelect,
}: {
  node: TreeNode;
  depth: number;
  selectedPath: string | null;
  onSelect: (path: string, contentRef: string) => void;
}) {
  const [open, setOpen] = useState(true);
  const indent = { paddingLeft: `${depth * 0.9 + 0.25}rem` };

  if (node.children !== undefined) {
    return (
      <li className="file-tree__node file-tree__node--folder">
        <button
          type="button"
          className="file-tree__folder"
          style={indent}
          data-testid={`folder-${node.path}`}
          aria-expanded={open}
          title={node.path}
          onClick={() => setOpen((o) => !o)}
        >
          <span className="file-tree__twisty" aria-hidden="true">
            {open ? "▾" : "▸"}
          </span>
          <span className="file-tree__name">{node.name}</span>
        </button>
        {open ? (
          <ul className="file-tree__children">
            {node.children.map((child) => (
              <TreeRow
                key={child.path}
                node={child}
                depth={depth + 1}
                selectedPath={selectedPath}
                onSelect={onSelect}
              />
            ))}
          </ul>
        ) : null}
      </li>
    );
  }

  const selected = selectedPath === node.path;
  // The full path, not just the leaf name: in a nested tree the leaf alone
  // (`index.ts`) is the least informative part of what the user is hovering.
  const title = node.state ? `${node.path} — ${STATE_LABEL[node.state]}` : node.path;
  return (
    <li className="file-tree__node file-tree__node--file">
      <button
        type="button"
        className={selected ? "file-tree__file file-tree__file--active" : "file-tree__file"}
        style={indent}
        data-testid={`file-${node.path}`}
        data-state={node.state}
        aria-current={selected ? "true" : undefined}
        title={title}
        onClick={() => onSelect(node.path, node.contentRef ?? "")}
      >
        {node.state ? (
          <span className={`file-tree__state file-tree__state--${node.state}`} aria-hidden="true">
            {STATE_GLYPH[node.state]}
          </span>
        ) : null}
        <span className="file-tree__name mono">{node.name}</span>
      </button>
    </li>
  );
}
