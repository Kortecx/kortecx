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
//! off-journal subsystem — see [`SKELETON`] for the agentic-app analogue).
//!
//! [`SKELETON`]: crate::SKELETON
//!
//! The framework is passed as its stable wire label (`"vite_react"` / `"next_js"` /
//! `"svelte"` / `"auto"`) so gateway-core stays free of the `kx-app` envelope types (it
//! keeps app bytes opaque — the [`crate::AppCatalog`] discipline). `"auto"` and any unknown
//! label resolve to Vite-React (the simplest dev server).

/// How one template file's body is produced.
pub enum FileSource {
    /// A fixed, byte-known body (content-addressed + advanced directly; no model call).
    Static(&'static str),
    /// A model-authored body: the scaffold write recipe fills it from the user's prompt.
    /// `role` is woven into the authoring prompt (the [`crate::SKELETON`] convention).
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
    "react-dom": "^18.3.1",
    "@kortecx/sdk": "^0.1.1"
  },
  "devDependencies": {
    "@vitejs/plugin-react": "^4.3.1",
    "vite": "^5.4.0",
    "typescript": "^5.4.5",
    "@types/react": "^18.3.3",
    "@types/react-dom": "^18.3.0"
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

// The kortecx hosted-app supervisor runs this on a loopback port and opens
// http://127.0.0.1:<port>/ directly — there is no reverse proxy, so the app is
// reached at the origin root. `strictPort` fails loudly instead of silently drifting.
export default defineConfig({
  plugins: [react()],
  server: { host: "127.0.0.1", strictPort: true },
});
"#,
        ),
    },
    // Test files are EXCLUDED from the project's own type-check. A served app never loads
    // them, but the model reliably authors an idiomatic `App.test.tsx` importing `vitest` +
    // `@testing-library/react` — neither of which this template installs — so the serve-time
    // `tsc --noEmit` gate would block a perfectly runnable app on files the browser never
    // sees. Found live: 2 of 3 scaffolded apps were held back by exactly this.
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
  "include": ["src"],
  "exclude": ["**/*.test.ts", "**/*.test.tsx", "**/*.spec.ts", "**/*.spec.tsx"]
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
        // `import.meta.env` needs Vite's ambient types, or the serve-time `tsc --noEmit` gate
        // rejects `src/kx.ts`. Vite ships them under `vite/client`.
        path: "src/vite-env.d.ts",
        source: FileSource::Static("/// <reference types=\"vite/client\" />\n"),
    },
    TemplateFile {
        // The runtime client, written for the model rather than by it. Cross-file contract
        // drift is the known way scaffolded hosted apps break, so the one file that talks to
        // the runtime is FIXED and tsc-clean, and the authoring prompt tells the model to
        // import from it and only call it. `import.meta.env.VITE_KX_*` is filled by the
        // supervisor's `.env.local` at start — the page never hard-codes an endpoint or a
        // token, and a build with no runtime simply gets an unconfigured client.
        path: "src/kx.ts",
        source: FileSource::Static(
            r#"// The kortecx runtime client for this hosted app. Do not edit — the supervisor fills
// VITE_KX_ENDPOINT / VITE_KX_TOKEN at start, and this app may only run the apps its
// envelope declared in references.apps.
import { KxClient } from "@kortecx/sdk/web";

const endpoint = import.meta.env.VITE_KX_ENDPOINT ?? "";
const token = import.meta.env.VITE_KX_TOKEN ?? "";

/** True when this app was served by a running kortecx gateway (env is present). */
export const kxConfigured: boolean = endpoint !== "" && token !== "";

/** The runtime client. Present only when configured; guard on {@link kxConfigured}. */
export const kx: KxClient | null = kxConfigured
  ? new KxClient(endpoint, { token })
  : null;

/**
 * Run one of this app's declared apps and wait for its answer as text.
 *
 * The handle must be one this hosted app declared in references.apps — the gateway refuses
 * anything else. Returns the committed result as a string, or throws with the runtime's
 * message.
 */
export async function runApp(
  handle: string,
  args: Record<string, string> = {},
): Promise<string> {
  if (!kx) throw new Error("the kortecx runtime is not configured for this app");
  // `wait: true` resolves to a settled Result (whose `.text` is the committed answer), never
  // the un-awaited Run handle.
  const result = await kx.runApp(handle, { args, wait: true });
  return "text" in result ? (result.text ?? "") : "";
}
"#,
        ),
    },
    TemplateFile {
        path: "src/App.tsx",
        source: FileSource::Authored {
            role: "the main React component in `src/App.tsx` implementing the web app the user \
                   described. Export a default React function component. Use `react`, and — when \
                   the app should run one of its kortecx apps — import `{ runApp, kxConfigured }` \
                   from `./kx` and call `runApp(<handle>, <args>)`; do NOT construct a client or \
                   import from `@kortecx/sdk` directly (the `./kx` module already did). Use \
                   inline styles or the classes in index.css. Keep it a single self-contained \
                   component that renders immediately.",
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
  },
  "devDependencies": {
    "typescript": "^5.4.5",
    "@types/node": "^20.14.0",
    "@types/react": "^18.3.3",
    "@types/react-dom": "^18.3.0"
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
  "exclude": [
    "node_modules",
    "**/*.test.ts",
    "**/*.test.tsx",
    "**/*.spec.ts",
    "**/*.spec.tsx"
  ]
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

/// The Vite + Svelte (TypeScript) SPA template — a lightweight React alternative with the
/// SAME dev-server shape (Vite root server), so the supervisor and `dev_command_args`
/// treat it exactly like Vite-React. `npm run dev` launches Vite; the supervisor appends
/// `-- --port <p>`.
const SVELTE: &[TemplateFile] = &[
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
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "^3.1.1",
    "svelte": "^4.2.18",
    "typescript": "^5.4.5",
    "vite": "^5.4.0"
  }
}
"#,
        ),
    },
    TemplateFile {
        path: "svelte.config.js",
        source: FileSource::Static(
            r#"import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  preprocess: vitePreprocess(),
};
"#,
        ),
    },
    TemplateFile {
        path: "vite.config.ts",
        source: FileSource::Static(
            r#"import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The kortecx hosted-app supervisor runs this on a loopback port and opens
// http://127.0.0.1:<port>/ directly — there is no reverse proxy, so the app is
// reached at the origin root. `strictPort` fails loudly instead of silently drifting.
export default defineConfig({
  plugins: [svelte()],
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
    "target": "ESNext",
    "useDefineForClassFields": true,
    "module": "ESNext",
    "lib": ["ESNext", "DOM", "DOM.Iterable"],
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "strict": true,
    "skipLibCheck": true
  },
  "include": ["src"],
  "exclude": ["**/*.test.ts", "**/*.test.tsx", "**/*.spec.ts", "**/*.spec.tsx"]
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
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
"#,
        ),
    },
    TemplateFile {
        path: "src/vite-env.d.ts",
        source: FileSource::Static(
            r#"/// <reference types="svelte" />
/// <reference types="vite/client" />
"#,
        ),
    },
    TemplateFile {
        path: "src/main.ts",
        source: FileSource::Static(
            r#"import App from "./App.svelte";
import "./app.css";

const app = new App({ target: document.getElementById("app")! });

export default app;
"#,
        ),
    },
    TemplateFile {
        path: "src/app.css",
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
        path: "src/App.svelte",
        source: FileSource::Authored {
            role: "the main Svelte component in `src/App.svelte` implementing the web app the \
                   user described. Use ONLY `svelte` (no extra npm dependencies): a \
                   `<script lang=\"ts\">` block, the markup, and a `<style>` block. Keep it a \
                   single self-contained component that renders immediately.",
            default: r#"<script lang="ts">
  let count = 0;
</script>

<main>
  <h1>Your hosted app is live</h1>
  <p>
    This is a Vite + Svelte app scaffolded and served by the kortecx runtime. Edit
    <code>src/App.svelte</code> to build it out.
  </p>
  <button on:click={() => (count += 1)}>clicked {count} times</button>
</main>

<style>
  main {
    max-width: 680px;
    margin: 4rem auto;
    padding: 0 1.5rem;
  }
</style>
"#,
        },
    },
    TemplateFile {
        path: "README.md",
        source: FileSource::Authored {
            role: "a concise README: what this hosted Svelte web app does and that it runs \
                   locally via the kortecx runtime (Run button) or `npm install && npm run dev`.",
            default: "# Hosted app\n\nA Vite + Svelte app scaffolded and served by the kortecx \
                      runtime. Run it with the Run button, or `npm install && npm run dev`.\n",
        },
    },
];

/// True iff `framework` is the Next.js label. Any other label (incl. `"auto"`, `"svelte"`,
/// and the empty string) uses the Vite root dev server (`--port <p>`) — see
/// [`dev_command_args`]; only Next.js needs the distinct `-p <p>` flag.
#[must_use]
fn is_next(framework: &str) -> bool {
    framework == "next_js"
}

/// The template file set for `framework` (`"vite_react"` / `"next_js"` / `"svelte"` /
/// `"auto"`). `"auto"`/unknown resolves to Vite-React (the host resolves `auto` to a
/// concrete framework via a model pre-step BEFORE scaffolding; this is the safe fallback).
#[must_use]
pub fn template(framework: &str) -> &'static [TemplateFile] {
    match framework {
        "next_js" => NEXT_JS,
        "svelte" => SVELTE,
        _ => VITE_REACT,
    }
}

/// The stable path set for `framework` (the order files are written + the scaffold
/// status reports). Deterministic — the tests assert exactly these.
#[must_use]
pub fn template_paths(framework: &str) -> Vec<&'static str> {
    template(framework).iter().map(|f| f.path).collect()
}

/// The framework's ENTRY component — the one authored file the template's static entry
/// imports by name, and therefore the file that decides whether the served page is the
/// user's app or the template's placeholder.
///
/// Vite-React's `src/main.tsx` does `import App from "./App.tsx"`; Next's `app/layout.tsx`
/// renders `app/page.tsx`; Svelte's `src/main.ts` imports `./App.svelte`. Each is `Static`,
/// so it is ALWAYS written and always imports this exact path.
///
/// Named explicitly rather than derived from "the first `Authored` file that is not the
/// README", because the derivation would silently follow a template reordering while this
/// contract — what the static entry imports — would not have moved. The planner contract in
/// `kx-gateway::manifest` already tells the model it MUST emit this path; this is the value
/// the host uses to make that true instead of merely requested.
#[must_use]
pub fn entry_path(framework: &str) -> &'static str {
    match framework {
        "next_js" => "app/page.tsx",
        "svelte" => "src/App.svelte",
        _ => "src/App.tsx",
    }
}

/// The template's authoring role for `path`, if that path is a model-authored template
/// file. Lets the host re-plan a template file (notably [`entry_path`]) using the template's
/// OWN role text, so an injected file is authored to the same contract as a planned one.
#[must_use]
pub fn authored_role(framework: &str, path: &str) -> Option<&'static str> {
    template(framework).iter().find_map(|f| match f.source {
        FileSource::Authored { role, .. } if f.path == path => Some(role),
        _ => None,
    })
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

/// The BUILD command for `framework` — `npm run build` for all three templates (every
/// `package.json` above declares that script). Returns the args AFTER `npm`.
///
/// Only the production serve lane runs this; the dev lane never builds.
#[must_use]
pub fn build_command_args(_framework: &str) -> Vec<String> {
    vec!["run".into(), "build".into()]
}

/// The command that serves the BUILT output for `framework`, the production counterpart
/// of [`dev_command_args`]. Returns the args AFTER `npm`.
///
/// The scripts genuinely differ, which is why this cannot be one string: Vite and Svelte
/// declare `preview` (and need `--host` pinned, since `vite preview` does not inherit the
/// dev server's `server.host` from `vite.config.ts`), while **Next declares no `preview`
/// at all** — `next start` is its production server, and it takes `-p`.
#[must_use]
pub fn preview_command_args(framework: &str, port: u16) -> Vec<String> {
    let port = port.to_string();
    if is_next(framework) {
        vec!["run".into(), "start".into(), "--".into(), "-p".into(), port]
    } else {
        vec![
            "run".into(),
            "preview".into(),
            "--".into(),
            "--port".into(),
            port,
            "--host".into(),
            "127.0.0.1".into(),
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
                "src/vite-env.d.ts",
                "src/kx.ts",
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
    fn svelte_path_set_is_stable() {
        assert_eq!(
            template_paths("svelte"),
            vec![
                "package.json",
                "svelte.config.js",
                "vite.config.ts",
                "tsconfig.json",
                "index.html",
                "src/vite-env.d.ts",
                "src/main.ts",
                "src/app.css",
                "src/App.svelte",
                "README.md",
            ]
        );
    }

    #[test]
    fn auto_and_unknown_resolve_to_vite_react() {
        assert_eq!(template_paths("auto"), template_paths("vite_react"));
        assert_eq!(template_paths(""), template_paths("vite_react"));
        // An unknown label still falls back to Vite-React; `"svelte"` is now its own set.
        assert_eq!(template_paths("preact"), template_paths("vite_react"));
    }

    #[test]
    fn every_static_package_json_parses_and_has_a_dev_script() {
        for fw in ["vite_react", "next_js", "svelte"] {
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
    fn every_template_declares_typescript_devdeps() {
        // Every template ships TypeScript source, so its package.json MUST declare
        // `typescript` as a devDependency (+ the framework's own type stubs). Otherwise the
        // dev server auto-installs them mid-startup — which crashed `next dev` with a
        // require-hook TypeError (found in live hosted-app testing). Regression guard.
        for fw in ["vite_react", "next_js", "svelte"] {
            let pkg = template(fw)
                .iter()
                .find(|f| f.path == "package.json")
                .expect("a package.json");
            let FileSource::Static(body) = pkg.source else {
                panic!("package.json must be static");
            };
            let v: serde_json::Value = serde_json::from_str(body).expect("package.json parses");
            let dev = &v["devDependencies"];
            assert!(
                dev["typescript"].is_string(),
                "{fw}: package.json must declare a typescript devDependency"
            );
            // The framework's own type stubs: React apps need `@types/react`; a Svelte app
            // gets its types from the `svelte` devDependency itself.
            if fw == "svelte" {
                assert!(
                    dev["svelte"].is_string(),
                    "{fw}: package.json must declare a svelte devDependency"
                );
            } else {
                assert!(
                    dev["@types/react"].is_string(),
                    "{fw}: package.json must declare @types/react"
                );
            }
        }
    }

    #[test]
    fn every_template_tsconfig_excludes_test_globs() {
        // The model reliably authors an idiomatic `App.test.tsx` importing `vitest` +
        // `@testing-library/react`, and no template installs either. Without these excludes
        // the serve-time `tsc --noEmit` gate blocks a perfectly runnable app over files the
        // browser never loads — observed live on 2 of 3 scaffolded apps. Regression guard on
        // the CONSTANTS, which is what a newly scaffolded project copies.
        //
        // Note the blast radius this guard does NOT cover: a template edit reaches only NEWLY
        // scaffolded apps. An existing app replays its original template bytes from its own
        // content-addressed branch.
        for fw in ["vite_react", "next_js", "svelte"] {
            let ts = template(fw)
                .iter()
                .find(|f| f.path == "tsconfig.json")
                .expect("a tsconfig.json");
            let FileSource::Static(body) = ts.source else {
                panic!("tsconfig.json must be static");
            };
            let v: serde_json::Value = serde_json::from_str(body).expect("tsconfig.json parses");
            let excludes: Vec<&str> = v["exclude"]
                .as_array()
                .unwrap_or_else(|| panic!("{fw}: tsconfig.json must carry an exclude array"))
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect();
            for glob in [
                "**/*.test.ts",
                "**/*.test.tsx",
                "**/*.spec.ts",
                "**/*.spec.tsx",
            ] {
                assert!(
                    excludes.contains(&glob),
                    "{fw}: tsconfig.json must exclude {glob} (got {excludes:?})"
                );
            }
        }
    }

    #[test]
    fn exactly_the_page_and_readme_are_authored() {
        // The visible page + README are model-authored; everything else is fixed.
        for fw in ["vite_react", "next_js", "svelte"] {
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
    fn entry_path_is_the_file_the_static_entry_actually_imports() {
        // The contract this const encodes: the template's STATIC entry file imports the
        // entry component BY NAME, so if the branch has no such file the served page is the
        // template's placeholder under the user's App name. Assert the import, not the
        // string — a template edit that renamed the entry would otherwise pass.
        for (fw, static_entry, entry) in [
            ("vite_react", "src/main.tsx", "src/App.tsx"),
            ("next_js", "app/layout.tsx", "app/page.tsx"),
            ("svelte", "src/main.ts", "src/App.svelte"),
        ] {
            assert_eq!(entry_path(fw), entry, "{fw}");
            // The entry is a real, model-AUTHORED file of that template.
            assert!(
                authored_role(fw, entry).is_some(),
                "{fw}: {entry} must be an authored template file"
            );
            // Next renders `page.tsx` by routing convention rather than by import; the other
            // two name it in an import statement.
            if fw != "next_js" {
                let src = template(fw)
                    .iter()
                    .find(|f| f.path == static_entry)
                    .expect("the static entry");
                let FileSource::Static(body) = src.source else {
                    panic!("{fw}: {static_entry} must be static");
                };
                let stem = entry.rsplit('/').next().unwrap();
                let base = stem.strip_suffix(".tsx").unwrap_or(stem);
                assert!(
                    body.contains(base),
                    "{fw}: {static_entry} must import {entry}"
                );
            }
        }
        // Unknown / "auto" resolves to Vite-React, matching `template`'s own fallback.
        assert_eq!(entry_path("auto"), "src/App.tsx");
        assert_eq!(entry_path("wat"), "src/App.tsx");
    }

    #[test]
    fn authored_role_is_none_for_static_and_unknown_paths() {
        assert!(authored_role("vite_react", "package.json").is_none());
        assert!(authored_role("vite_react", "src/nope.tsx").is_none());
        assert!(authored_role("vite_react", "README.md").is_some());
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
        // Svelte uses the Vite root dev server, so it takes the same `--port` flag.
        assert_eq!(
            dev_command_args("svelte", 4321),
            vec!["run", "dev", "--", "--port", "4321"]
        );
    }
}
