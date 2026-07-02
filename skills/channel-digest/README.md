# channel-digest

A reference `kortecx.skill/v1` pack over the bundled Discord connector
(`kx-connector-discord`): list channels → read recent messages → digest.
Read-only by construction — the wish set carries no `discord/send_message`.

A skill is **declarative**: instructions + a tool grant-**wish** set. The wishes
resolve only if the caller has connected Discord and the serve can fire the
dialed tools — a skill on its own grants nothing.

## Use it

```sh
kx connections add --provider discord
kx skills add --dir skills/channel-digest
kx app new standup-digest --from-blueprint blueprint.json --skill channel-digest
kx app run standup-digest --arg guild="engineering"
```

Conformance: `just test-skill skills/channel-digest`.
