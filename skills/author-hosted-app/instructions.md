# Authoring a hosted app

A **hosted app** is a real web application — a Vite-React or Next.js project the
runtime **scaffolds into a file tree**, installs, and **serves on a local port** you
open in a browser tab. Unlike a scheduled app, it carries no agentic blueprint; it is
the *face* the user interacts with.

## Procedure

1. **Choose the framework.**
   - **Vite-React** (default) — a single-page app; the fastest, simplest dev server.
     Choose this unless you specifically need server rendering.
   - **Next.js** — choose only when the app needs SSR, route handlers, or file-based
     routing.

2. **Design the project.** The runtime writes the fixed project shell (package
   manifest, build config, entry HTML/layout) for you; you author the **visible
   page** (`src/App.tsx` for Vite, `app/page.tsx` for Next) that implements what the
   user asked for. Use only `react` (and `next` for Next.js) — no extra npm
   dependencies — and make it render immediately.

3. **How it runs.** The gateway's hosted-app supervisor installs dependencies and
   starts the dev server on a **local loopback port**. The OSS boundary is a *local*
   dev server — never a public URL (that is Cloud). Say so plainly.

4. **How it comes alive.** A hosted app reaches the runtime's agentic capabilities
   only as an external client through the governed request seam, under a warrant — it
   does **not** carry baked tool grants. Until that wiring is present, treat the app
   as a self-contained local experience; do **not** claim it can reach live user data
   or the internet at will.

5. **Output contract.** A saved `kortecx.experience/v1` manifest plus a real project
   file tree in the app's branch — runnable with the console Run button (or
   `npm install && npm run dev`).
