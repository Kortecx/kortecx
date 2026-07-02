# research-summarize

A reference `kortecx.skill/v1` pack: grounded question-answering over the
runtime's own data plane — `retrieve@1` (dataset RAG) + `fs-read@1` (confined
file reads). No connector or external credential required.

A skill is **declarative**: instructions + a tool grant-**wish** set. Attaching
it grants nothing by itself — at `RunApp` the server intersects the wish against
the caller's grants and the live broker (`wish ∩ grants ∩ fireable`), and only
the survivors are granted to the App's entry agentic step.

## Use it

```sh
kx skills add --dir skills/research-summarize
kx app new my-researcher --from-blueprint blueprint.json --skill research-summarize
kx app run my-researcher --arg question="What does the design doc say about recovery?"
```

Conformance: `just test-skill skills/research-summarize`.
