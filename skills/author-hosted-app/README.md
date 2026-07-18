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
kx app new my-site --skill author-hosted-app
# then Run it from the console (the runtime installs + serves it on a local port)
```

Conformance: `just test-skill skills/author-hosted-app`.
