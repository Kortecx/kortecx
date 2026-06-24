/**
 * POC-5d: the App project file tree (left pane of the App detail view). Renders
 * the hierarchical {@link TreeNode} list from {@link buildFileTree}: folders are
 * expand/collapse buttons (aria-expanded), files are selectable buttons carrying
 * a {@link DigestChip} of their content ref. Token-only colours, keyboard-
 * accessible, both themes. Selection is owned by the parent (the selected path is
 * highlighted); the tree itself is presentational.
 */

import { useState } from "react";
import type { TreeNode } from "../../lib/file-tree";
import { DigestChip } from "../DigestChip";

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
  return (
    <li className="file-tree__node file-tree__node--file">
      <button
        type="button"
        className={selected ? "file-tree__file file-tree__file--active" : "file-tree__file"}
        style={indent}
        data-testid={`file-${node.path}`}
        aria-current={selected ? "true" : undefined}
        onClick={() => onSelect(node.path, node.contentRef ?? "")}
      >
        <span className="file-tree__name mono">{node.name}</span>
        {node.contentRef ? <DigestChip hex={node.contentRef} label={node.path} /> : null}
      </button>
    </li>
  );
}
