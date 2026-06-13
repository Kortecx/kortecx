import { themes as prismThemes } from "prism-react-renderer";
import type { Config } from "@docusaurus/types";
import type * as Preset from "@docusaurus/preset-classic";

// kortecx documentation site — https://kortecx.com/docs
// Docs-focused classic preset, dark theme by default to match the product console.
const config: Config = {
  title: "Kortecx",
  tagline: "The durable runtime for AI agents — Knowledge → Intelligence.",
  favicon: "img/favicon.svg",

  // Production URL. Served under /docs/ so it sits beside the marketing site at kortecx.com.
  url: "https://kortecx.com",
  baseUrl: "/docs/",

  organizationName: "Kortecx",
  projectName: "kortecx",

  // Surface broken-link problems at build time — the gate depends on it.
  onBrokenLinks: "throw",

  markdown: {
    hooks: {
      onBrokenMarkdownLinks: "throw",
    },
  },

  i18n: {
    defaultLocale: "en",
    locales: ["en"],
  },

  presets: [
    [
      "classic",
      {
        docs: {
          // Docs at the site root (no /docs/docs nesting under baseUrl).
          routeBasePath: "/",
          sidebarPath: "./sidebars.ts",
          editUrl: "https://github.com/Kortecx/kortecx/tree/main/docs/site/",
        },
        // Docs-focused: the blog is disabled.
        blog: false,
        theme: {
          customCss: "./src/css/custom.css",
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    image: "img/kortecx-social-card.svg",
    colorMode: {
      defaultMode: "dark",
      respectPrefersColorScheme: false,
    },
    navbar: {
      title: "Kortecx",
      logo: {
        alt: "Kortecx",
        src: "img/logo.svg",
        // The docs are served at baseUrl /docs/ with the intro at /docs/intro;
        // point the brand link at a real page (there is no /docs/ landing route).
        href: "/intro",
      },
      items: [
        {
          type: "docSidebar",
          sidebarId: "docs",
          position: "left",
          label: "Docs",
        },
        {
          href: "https://kortecx.com",
          label: "kortecx.com",
          position: "right",
        },
        {
          href: "https://github.com/Kortecx/kortecx",
          label: "GitHub",
          position: "right",
        },
      ],
    },
    footer: {
      style: "dark",
      links: [
        {
          title: "Docs",
          items: [
            { label: "Introduction", to: "/intro" },
            { label: "Quickstart", to: "/quickstart" },
            { label: "Concepts", to: "/concepts" },
            { label: "Chains DSL", to: "/chains/dsl-reference" },
          ],
        },
        {
          title: "Community",
          items: [
            {
              label: "Issues",
              href: "https://github.com/Kortecx/kortecx/issues",
            },
            {
              label: "Contributing",
              href: "https://github.com/Kortecx/kortecx/blob/main/CONTRIBUTING.md",
            },
          ],
        },
        {
          title: "More",
          items: [
            { label: "Website", href: "https://kortecx.com" },
            { label: "GitHub", href: "https://github.com/Kortecx/kortecx" },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Kortecx. Licensed under Apache-2.0.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ["bash", "python", "json", "toml", "rust"],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
