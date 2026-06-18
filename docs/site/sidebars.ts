import type { SidebarsConfig } from "@docusaurus/plugin-content-docs";

// A hand-authored sidebar so the reading order is intentional (rather than
// auto-generated). Each id maps to a docs/<id>.md file.
const sidebars: SidebarsConfig = {
  docs: [
    {
      type: "category",
      label: "Getting started",
      collapsed: false,
      items: ["intro", "quickstart", "concepts"],
    },
    {
      type: "category",
      label: "Chains",
      collapsed: false,
      items: [
        "chains/dsl-reference",
        "chains/python",
        "chains/typescript",
      ],
    },
    {
      type: "category",
      label: "Platform",
      collapsed: false,
      items: [
        "chat",
        "workflows",
        "blueprint-builder",
        "agent-runner",
        "security",
        "tools",
        "context",
        "models",
        "datasets",
        "scripts",
        "reading-run-outputs",
        "rerun-with-changes",
        "observability",
      ],
    },
    {
      type: "category",
      label: "Reference",
      collapsed: false,
      items: ["api-reference"],
    },
  ],
};

export default sidebars;
