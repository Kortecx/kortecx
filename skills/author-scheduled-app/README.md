# author-scheduled-app

A reference `kortecx.skill/v1` pack that steers the model to author a **scheduled**
(functional) app: a durable blueprint that runs unattended on a trigger and wires
the tools, connections and integrations it needs to do a real job.

A skill is **declarative**: instructions + a tool grant-**wish** set. The wishes here
are read/draft-biased across Gmail / Notion / Slack / Discord (plus `retrieve@1`) and
**deliberately omit every irreversible send** — an unattended run stages a send/post
for human approval, it never fires one silently. A skill on its own grants nothing;
the wishes resolve only if the caller connected the service and the serve can fire the
dialed tools.

## Use it

```sh
kx connections add --provider gmail          # connect any services the job needs
kx skills add --dir skills/author-scheduled-app
kx app new my-automation --skill author-scheduled-app
kx app run my-automation
```

Conformance: `just test-skill skills/author-scheduled-app`.
