# Docs API-reference pipeline

The Python and TypeScript SDK API references are **generated** into
`static/api/` so Docusaurus serves them as static assets at
`https://kortecx.com/docs/api/`. They are **not** committed (treat
`static/api/typescript/` and `static/api/python/` as build output; only
`static/api/index.html` is checked in).

> This pipeline is **scaffolded, not wired into CI**. The commands below are real
> but you must install the generators yourself; nothing here is added to any
> Rust / TS / Py product build.

## TypeScript — TypeDoc

```bash
# from docs/site/
npm install            # installs typedoc (a devDependency)
npm run api:ts         # typedoc --options typedoc.json  →  static/api/typescript/
```

Configured by [`../typedoc.json`](../typedoc.json). The entry point resolves
against the in-repo SDK at `bindings/typescript/src/index.ts`. If the SDK's
public entry point moves, update `entryPoints` there.

## Python — pdoc

```bash
# from docs/site/, with the kortecx SDK importable on the active interpreter:
pip install pdoc kortecx
npm run api:py         # pdoc -o static/api/python kortecx  →  static/api/python/
```

`pdoc` introspects the installed `kortecx` package, so the SDK must be importable
(`pip install kortecx`, or `pip install -e bindings/python`). No `protoc` is
needed — the SDK ships committed stubs.

## Wiring into a build (future)

To regenerate both references before a docs deploy:

```bash
npm run api:ts && npm run api:py && npm run build
```

When CI publishes the docs, run the two `api:*` steps before `docusaurus build`
so the generated references ship with the site. Until then, the
[API reference page](../docs/api-reference.md) links to the inline SDK READMEs as
the source of truth.
