# author-hosted-app

A reference `kortecx.skill/v1` pack that steers the model to author a **hosted**
(experience) app: a real Vite-React or Next.js web project the runtime scaffolds into
a branch file tree and serves on a local port.

A skill is **declarative**: instructions + a tool grant-**wish** set. The wishes here
are limited to the bundled read capabilities the scaffolder genuinely uses
(`retrieve@1`, `fs-read@1`). A hosted app fires **no** functional tools at run — it
reaches the runtime's capabilities only through the governed request seam, under a
warrant — so encoding hosted-lifecycle "tool" wishes here would misrepresent grants
the runtime does not resolve. When real hosted-lifecycle tools are registered and
fireable, this skill may wish for them.

## Use it

```sh
kx skills add --dir skills/author-hosted-app
```

Then create the App in the console's **New App** form (choose the **Hosted** kind and a
framework) or with the SDK's `.hosted(framework, …)` builder, and attach this skill. The
`kx app` CLI has no hosted-kind flag and `kx app new` requires `--from-blueprint <file>`,
so it cannot author a hosted App.

The hosted lane ships in the prebuilt binary and every `just serve*` recipe; **serving one
needs Node/npm on the host** (the supervisor runs `npm install` and a dev server).

Conformance: `just test-skill skills/author-hosted-app`.
