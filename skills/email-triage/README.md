# email-triage

A reference `kortecx.skill/v1` pack over the bundled Gmail connector
(`kx-connector-gmail`): search → read → **draft** (deliberately no
`gmail/send` wish — the propose-only posture; sending stays a human act).

A skill is **declarative**: instructions + a tool grant-**wish** set. The wishes
here resolve only if the caller has connected Gmail (`kx connections add
--provider gmail`) and the serve can fire the dialed tools — a skill on its own
grants nothing.

## Use it

```sh
kx connections add --provider gmail          # G1: connect Gmail (credential by ref)
kx skills add --dir skills/email-triage
kx app new inbox-triage --from-blueprint blueprint.json --skill email-triage
kx app run inbox-triage
```

Conformance: `just test-skill skills/email-triage`.
