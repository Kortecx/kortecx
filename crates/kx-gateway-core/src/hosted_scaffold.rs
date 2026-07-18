//! D213 Experience lane — framework project TEMPLATES for hosted-app scaffolding.
//!
//! A hosted (Experience) App is a real web project. Unlike the agentic [`SKELETON`]
//! (every file model-authored), a hosted project is mostly FIXED boilerplate (package
//! manifest, build config, entry HTML) with only the visible page(s) authored from the
//! user's prompt. So each template file is either:
//!
//! - [`FileSource::Static`] — a byte-known file the host content-addresses and advances
//!   directly (no model call; deterministic; CI-testable), or
//! - [`FileSource::Authored`] — a model-authored file written through the SAME per-file
//!   write recipe the agentic scaffold uses (`app-scaffold-write`).
//!
//! The STRUCTURE (the path set) is fixed per framework (the deterministic tests assert
//! it); only the authored files' CONTENT varies. The generated tree is materialized to
//! disk + `npm install`ed + dev-served by the host hosted-app supervisor (a separate,
//! off-journal subsystem — see [`crate::scaffold`] for the agentic-app analogue).
//!
//! [`SKELETON`]: crate::scaffold::SKELETON
//!
//! The framework is passed as its stable wire label (`"vite_react"` / `"next_js"` /
//! `"auto"`) so gateway-core stays free of the `kx-app` envelope types (it keeps app
//! bytes opaque — the [`crate::AppCatalog`] discipline). `"auto"` and any unknown label
//! resolve to Vite-React (the simplest dev server).

/// How one template file's body is produced.
pub enum FileSource {
    /// A fixed, byte-known body (content-addressed + advanced directly; no model call).
    Static(&'static str),
    /// A model-authored body: the scaffold write recipe fills it from the user's prompt.
    /// `role` is woven into the authoring prompt (the [`crate::scaffold`] convention).
    /// `default` is a byte-known, RUNNABLE fallback used when no model is served (a hosted
    /// project is always valid + servable even model-free; the model only enriches the page).
    Authored {
        /// A short role description of what the model should write into this file.
        role: &'static str,
        /// A working default body (model-free scaffolds + hermetic tests use this).
        default: &'static str,
    },
}

/// One template file: a stable manifest path + how its body is produced.
pub struct TemplateFile {
    /// The manifest path (stable — the deterministic tests assert exactly these).
    pub path: &'static str,
    /// The body source (static bytes or a model-authored role).
    pub source: FileSource,
}

/// The Vite + React (TypeScript) SPA template — the simplest dev server to supervise.
/// `npm run dev` launches Vite; the supervisor appends `-- --port <p>`.
const VITE_REACT: &[TemplateFile] = &[
    TemplateFile {
        path: "package.json",
        source: FileSource::Static(
            r#"{
  "name": "kortecx-hosted-app",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite --host 127.0.0.1",
    "build": "vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "@vitejs/plugin-react": "^4.3.1",
    "vite": "^5.4.0"
  }
}
"#,
        ),
    },
    TemplateFile {
        path: "vite.config.ts",
        source: FileSource::Static(
            r#"import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The kortecx hosted-app supervisor runs `npm run dev -- --port <p>` on a loopback
// port and reverse-proxies it. `strictPort` fails loudly instead of silently drifting.
export default defineConfig({
  plugins: [react()],
  server: { host: "127.0.0.1", strictPort: true },
});
"#,
        ),
    },
    TemplateFile {
        path: "tsconfig.json",
        source: FileSource::Static(
            r#"{
  "compilerOptions": {
    "target": "ES2020",
    "useDefineForClassFields": true,
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true
  },
  "include": ["src"]
}
"#,
        ),
    },
    TemplateFile {
        path: "index.html",
        source: FileSource::Static(
            r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>kortecx hosted app</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
"#,
        ),
    },
    TemplateFile {
        path: "src/main.tsx",
        source: FileSource::Static(
            r#"import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App.tsx";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
"#,
        ),
    },
    TemplateFile {
        path: "src/index.css",
        source: FileSource::Static(
            r#":root {
  color-scheme: light dark;
  font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
}
body {
  margin: 0;
  min-height: 100vh;
}
"#,
        ),
    },
    TemplateFile {
        path: "src/App.tsx",
        source: FileSource::Authored {
            role: "the main React component in `src/App.tsx` implementing the web app the user \
                   described. Export a default React function component. Use ONLY `react` (no \
                   extra npm dependencies) and inline styles or the classes in index.css. Keep \
                   it a single self-contained component that renders immediately.",
            default: r#"export default function App() {
  return (
    <main style={{ maxWidth: 680, margin: "4rem auto", padding: "0 1.5rem" }}>
      <h1>Your hosted app is live</h1>
      <p>
        This is a Vite + React app scaffolded and served by the kortecx runtime. Edit{" "}
        <code>src/App.tsx</code> to build it out.
      </p>
    </main>
  );
}
"#,
        },
    },
    TemplateFile {
        path: "README.md",
        source: FileSource::Authored {
            role: "a concise README: what this hosted web app does and that it runs locally via \
                   the kortecx runtime (Run button) or `npm install && npm run dev`.",
            default: "# Hosted app\n\nA Vite + React app scaffolded and served by the kortecx \
                      runtime. Run it with the Run button, or `npm install && npm run dev`.\n",
        },
    },
];

/// The Next.js (App Router, TypeScript) template. `npm run dev` launches `next dev`;
/// the supervisor appends `-- --port <p>`.
const NEXT_JS: &[TemplateFile] = &[
    TemplateFile {
        path: "package.json",
        source: FileSource::Static(
            r#"{
  "name": "kortecx-hosted-app",
  "private": true,
  "version": "0.1.0",
  "scripts": {
    "dev": "next dev",
    "build": "next build",
    "start": "next start"
  },
  "dependencies": {
    "next": "^14.2.5",
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  }
}
"#,
        ),
    },
    TemplateFile {
        path: "next.config.mjs",
        source: FileSource::Static(
            r"/** @type {import('next').NextConfig} */
const nextConfig = {};
export default nextConfig;
",
        ),
    },
    TemplateFile {
        path: "tsconfig.json",
        source: FileSource::Static(
            r#"{
  "compilerOptions": {
    "lib": ["dom", "dom.iterable", "esnext"],
    "allowJs": true,
    "skipLibCheck": true,
    "strict": true,
    "noEmit": true,
    "esModuleInterop": true,
    "module": "esnext",
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "jsx": "preserve",
    "incremental": true,
    "plugins": [{ "name": "next" }]
  },
  "include": ["next-env.d.ts", "**/*.ts", "**/*.tsx", ".next/types/**/*.ts"],
  "exclude": ["node_modules"]
}
"#,
        ),
    },
    TemplateFile {
        path: "next-env.d.ts",
        source: FileSource::Static(
            r#"/// <reference types="next" />
/// <reference types="next/image-types/global" />
"#,
        ),
    },
    TemplateFile {
        path: "app/layout.tsx",
        source: FileSource::Static(
            r#"import "./globals.css";
import type { ReactNode } from "react";

export const metadata = {
  title: "kortecx hosted app",
  description: "A hosted app served by the kortecx runtime.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
"#,
        ),
    },
    TemplateFile {
        path: "app/globals.css",
        source: FileSource::Static(
            r#":root {
  color-scheme: light dark;
  font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
}
body {
  margin: 0;
  min-height: 100vh;
}
"#,
        ),
    },
    TemplateFile {
        path: "app/page.tsx",
        source: FileSource::Authored {
            role: "the main page in `app/page.tsx` (Next.js App Router) implementing the web app \
                   the user described. Export a default React function component. Use ONLY `next` \
                   and `react` (no extra npm dependencies). Keep it a single self-contained \
                   component that renders immediately.",
            default: r#"export default function Page() {
  return (
    <main style={{ maxWidth: 680, margin: "4rem auto", padding: "0 1.5rem" }}>
      <h1>Your hosted app is live</h1>
      <p>
        This is a Next.js app scaffolded and served by the kortecx runtime. Edit{" "}
        <code>app/page.tsx</code> to build it out.
      </p>
    </main>
  );
}
"#,
        },
    },
    TemplateFile {
        path: "README.md",
        source: FileSource::Authored {
            role: "a concise README: what this hosted Next.js app does and that it runs locally \
                   via the kortecx runtime (Run button) or `npm install && npm run dev`.",
            default: "# Hosted app\n\nA Next.js app scaffolded and served by the kortecx runtime. \
                      Run it with the Run button, or `npm install && npm run dev`.\n",
        },
    },
];

/// True iff `framework` is the Next.js label. Any other label (incl. `"auto"` and the
/// empty string) is treated as Vite-React (the simplest, fastest dev server).
#[must_use]
fn is_next(framework: &str) -> bool {
    framework == "next_js"
}

/// The template file set for `framework` (`"vite_react"` / `"next_js"` / `"auto"`).
/// `"auto"`/unknown resolves to Vite-React (the host resolves `auto` to a concrete
/// framework via a model pre-step BEFORE scaffolding; this is the safe fallback).
#[must_use]
pub fn template(framework: &str) -> &'static [TemplateFile] {
    if is_next(framework) {
        NEXT_JS
    } else {
        VITE_REACT
    }
}

/// The stable path set for `framework` (the order files are written + the scaffold
/// status reports). Deterministic — the tests assert exactly these.
#[must_use]
pub fn template_paths(framework: &str) -> Vec<&'static str> {
    template(framework).iter().map(|f| f.path).collect()
}

/// The dev-server command for `framework`: `npm run dev -- --port <p>` for both, but the
/// port flag differs (Vite `--port`, Next `-p`). Returns the args AFTER `npm`.
#[must_use]
pub fn dev_command_args(framework: &str, port: u16) -> Vec<String> {
    let port = port.to_string();
    if is_next(framework) {
        // `next dev` takes `-p <port>`; the `--` separates npm's args from the script's.
        vec!["run".into(), "dev".into(), "--".into(), "-p".into(), port]
    } else {
        // Vite takes `--port <port>`.
        vec![
            "run".into(),
            "dev".into(),
            "--".into(),
            "--port".into(),
            port,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vite_react_path_set_is_stable() {
        assert_eq!(
            template_paths("vite_react"),
            vec![
                "package.json",
                "vite.config.ts",
                "tsconfig.json",
                "index.html",
                "src/main.tsx",
                "src/index.css",
                "src/App.tsx",
                "README.md",
            ]
        );
    }

    #[test]
    fn next_js_path_set_is_stable() {
        assert_eq!(
            template_paths("next_js"),
            vec![
                "package.json",
                "next.config.mjs",
                "tsconfig.json",
                "next-env.d.ts",
                "app/layout.tsx",
                "app/globals.css",
                "app/page.tsx",
                "README.md",
            ]
        );
    }

    #[test]
    fn auto_and_unknown_resolve_to_vite_react() {
        assert_eq!(template_paths("auto"), template_paths("vite_react"));
        assert_eq!(template_paths(""), template_paths("vite_react"));
        assert_eq!(template_paths("svelte"), template_paths("vite_react"));
    }

    #[test]
    fn every_static_package_json_parses_and_has_a_dev_script() {
        for fw in ["vite_react", "next_js"] {
            let pkg = template(fw)
                .iter()
                .find(|f| f.path == "package.json")
                .expect("a package.json");
            let FileSource::Static(body) = pkg.source else {
                panic!("package.json must be static");
            };
            let v: serde_json::Value = serde_json::from_str(body).expect("package.json parses");
            assert!(
                v["scripts"]["dev"].is_string(),
                "package.json needs a dev script"
            );
        }
    }

    #[test]
    fn exactly_the_page_and_readme_are_authored() {
        // The visible page + README are model-authored; everything else is fixed.
        for fw in ["vite_react", "next_js"] {
            let authored: Vec<&str> = template(fw)
                .iter()
                .filter(|f| matches!(f.source, FileSource::Authored { .. }))
                .map(|f| f.path)
                .collect();
            assert_eq!(authored.len(), 2, "{fw}: page + README are authored");
            assert!(authored.contains(&"README.md"));
        }
    }

    #[test]
    fn dev_command_args_carry_the_port() {
        assert_eq!(
            dev_command_args("vite_react", 4321),
            vec!["run", "dev", "--", "--port", "4321"]
        );
        assert_eq!(
            dev_command_args("next_js", 4321),
            vec!["run", "dev", "--", "-p", "4321"]
        );
    }
}
