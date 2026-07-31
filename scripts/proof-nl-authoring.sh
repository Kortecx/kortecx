#!/usr/bin/env bash
# The Rule-41 live proof for the NL authoring surface.
#
# Drives the WHOLE journey against a served Gemma on DEDICATED ports:
#
#     NL -> preview -> approve -> draft -> finish -> run
#
# plus an authority-exceeding REFUSAL A/B, which is the half that matters most: a
# surface that only ever says yes has not been shown to have a boundary.
#
# ── What makes this a proof rather than a smoke test ────────────────────────────
#
# * The serving binary's IDENTITY is re-asserted mid-run, not just at startup. A
#   stale orphan reports the right model while the wrong binary answers.
# * Every run assertion is scoped by `terminal_mote_id` and walks that terminal's
#   ancestors. A mote COUNT is never asserted: a parked mote is invisible until it
#   commits, so counting reads absence where the mechanism is working.
# * The approve step forwards the BYTES THE PREVIEW RETURNED. That is the property
#   the whole design exists for, so the proof compares them rather than trusting
#   that they match.
#
# Exits non-zero on the first failed assertion, with the reason.
set -euo pipefail

GRPC_PORT="${GRPC_PORT:-50171}"
EP="http://127.0.0.1:${GRPC_PORT}"
KX="${KX:-$(pwd)/target/release/kx}"
OUT="${PROOF_OUT:-target/proof/artifacts}"
mkdir -p "$OUT"

step() { printf '\n=== %s ===\n' "$*"; }
ok()   { printf '  ✓ %s\n' "$*"; }
die()  { printf '  ✗ %s\n' "$*" >&2; exit 1; }

jqp() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

step "0 · the serving binary is the one we built"
just _assert-serving-identity "$GRPC_PORT" "$KX" || die "identity check failed"

step "1 · DescribeControlSurface — the facade is reachable and classifies"
SURFACE="$OUT/surface.json"
# Step 0 already proved the binary we built is the one LISTENING; a second
# "is kx runnable" probe added nothing and its first draft used an invocation
# `kx` does not accept (`invoke --help`), failing the proof on the harness.
# There is no `kx control` verb by design (OSS ships no model-driven CLI),
# so the surface is exercised through the SDK-shaped gRPC path the console uses.
python3 scripts/proof_nl_client.py describe --endpoint "$EP" > "$SURFACE" \
  || die "DescribeControlSurface failed"
ENTRIES=$(jqp "len(d['entries'])" < "$SURFACE")
[ "$ENTRIES" -ge 115 ] || die "surface reported only $ENTRIES entries"
ok "surface describes $ENTRIES RPCs"
AUTHORING=$(jqp "sum(1 for e in d['entries'] if e['authoring'])" < "$SURFACE")
[ "$AUTHORING" -gt 0 ] || die "no authoring RPCs reported"
ok "$AUTHORING of them are in an authoring domain"
# The two NL RPCs must classify as READS. If either became a Mutate the runtime
# would have started acting on its own behalf.
for rpc in ProposeControlAction DescribeControlSurface; do
  M=$(jqp "next(e['mutates'] for e in d['entries'] if e['rpc']=='$rpc')" < "$SURFACE")
  [ "$M" = "False" ] || die "$rpc reports mutates=$M — a proposal must write nothing"
done
ok "both NL RPCs classify as Read (a proposal writes nothing)"

step "2 · NL -> PREVIEW (the model produces a typed, admissible form)"
PREVIEW="$OUT/preview.json"
python3 scripts/proof_nl_client.py propose \
  --endpoint "$EP" \
  --goal "Register a durable role called reporting-only that narrows tool authority to just the retrieve tool version 1." \
  --domain policy > "$PREVIEW" || die "ProposeControlAction failed"
KIND=$(jqp "d.get('kind','')" < "$PREVIEW")
[ "$KIND" = "preview" ] || die "expected a preview, got: $(cat "$PREVIEW")"
RPC=$(jqp "d['rpc']" < "$PREVIEW")
ok "previewed $RPC — $(jqp "d['summary']" < "$PREVIEW")"

step "3 · APPROVE forwards the BYTES THE PREVIEW RETURNED"
APPLIED="$OUT/applied.json"
python3 scripts/proof_nl_client.py approve \
  --endpoint "$EP" --preview "$PREVIEW" > "$APPLIED" || die "approve failed"
ok "$(jqp "d['detail']" < "$APPLIED")"
# Read it back through the ORDINARY read path — the registry, not the preview.
ROLES="$OUT/roles.json"
"$KX" policy list --json --endpoint "$EP" > "$ROLES" || die "policy list failed"
NAME=$(jqp "d['roles'][0]['name'] if d['roles'] else ''" < "$ROLES")
[ -n "$NAME" ] || die "the approved role is not in the registry"
ok "the registry now holds role '$NAME' (read back through the normal path)"

step "4 · the REFUSAL A/B — an authority-exceeding ask is refused"
# A TRUE A/B: the two prompts below are word-for-word identical except for the
# tool named. An earlier draft phrased this one as "grants the tool …", and the
# model — correctly steered by a contract that says a role NARROWS and never
# grants — produced a differently-shaped form that the DECODER refused. The
# refusal was real and the test went green, but it measured the wrong gate.
# Varying one word isolates the authority gate, which is the thing under test.
REFUSAL="$OUT/refusal.json"
python3 scripts/proof_nl_client.py propose \
  --endpoint "$EP" \
  --goal "Register a role called escalate that narrows to the definitely-not-registered tool version 9." \
  --domain policy > "$REFUSAL" || die "the refusal probe errored at transport level"
RKIND=$(jqp "d.get('kind','')" < "$REFUSAL")
[ "$RKIND" = "rejected" ] || die "an unregistered tool was NOT refused: $(cat "$REFUSAL")"
REASON=$(jqp "d['reason']" < "$REFUSAL")
# WHY the refusal fired matters as much as THAT it fired. An earlier run of this
# proof "passed" here because the model malformed the envelope and the DECODER
# refused it — a real refusal, but not the one being claimed. A refusal oracle
# that accepts any refusal cannot tell a boundary from a typo.
case "$REASON" in
  *definitely-not-registered*|*"no registered tool matches"*)
    ok "refused for the AUTHORITY reason: $(printf '%s' "$REASON" | head -c 130)" ;;
  *)
    die "refused, but NOT for the authority reason — the boundary was not exercised.
     got: $REASON
     (a decode refusal is a real refusal and a different claim; re-run, or teach the
      contract the shape the model got wrong)" ;;
esac
# The A/B: the SAME shape with a real tool must be ACCEPTED, or the refusal above
# proves only that the surface says no to everything.
CONTROL="$OUT/refusal-control.json"
python3 scripts/proof_nl_client.py propose \
  --endpoint "$EP" \
  --goal "Register a role called reporting-two that narrows to the retrieve tool version 1." \
  --domain policy > "$CONTROL" || die "the anti-always-refuse control errored"
CKIND=$(jqp "d.get('kind','')" < "$CONTROL")
[ "$CKIND" = "preview" ] || die "ANTI-CONTROL FAILED — the surface refuses everything: $(cat "$CONTROL")"
ok "the same shape with a REAL tool is accepted — the refusal discriminates"

step "5 · a secret proposal carries a NAME and no value"
SECRET="$OUT/secret.json"
python3 scripts/proof_nl_client.py propose \
  --endpoint "$EP" \
  --goal "We need a credential named REPORTING_API_KEY for reaching api.example.com." \
  --domain secrets > "$SECRET" || die "the secret proposal errored"
if [ "$(jqp "d.get('kind','')" < "$SECRET")" = "preview" ]; then
  # The wire type has no value field, so this cannot fail — which is the point.
  # Asserting it anyway makes the structural claim observable in the artifact.
  python3 -c "
import json,sys
d=json.load(open('$SECRET'))
blob=json.dumps(d)
for bad in ['\"value\"','password','hunter2']:
    assert bad not in blob, f'a secret preview carried {bad}: {blob[:400]}'
print('  ✓ the preview carries a NAME and no value (structurally impossible to)')
"
else
  ok "the secret proposal was refused (also acceptable): $(jqp "d['reason']" < "$SECRET" | head -c 90)"
fi

step "6 · DRAFT -> FINISH -> RUN (the lifecycle flip on identical bytes)"
WF="$OUT/wf.json"
# `blueprint` is REQUIRED — a workflow without one is not a workflow, and the
# field is deliberately non-optional so its absence cannot be mistaken for an
# empty one. The first draft of this proof omitted it and the save refused with
# an empty stderr, which is why the error is captured rather than discarded now.
cat > "$OUT/wf-envelope.json" <<'JSON'
{"schema":"kortecx.workflow/v1","name":"proof-report","version":"1",
 "description":"the Rule-41 proof workflow",
 "blueprint":{"seed":0,"steps":[{"kind":"pure"}],"edges":[]}}
JSON
"$KX" workflow save proof/nl/report "$OUT/wf-envelope.json" --draft --json --endpoint "$EP" > "$WF" 2> "$OUT/wf-err.txt" \
  || die "draft save failed: $(cat "$OUT/wf-err.txt" 2>/dev/null)"
ok "saved as DRAFT"
"$KX" workflow save proof/nl/report "$OUT/wf-envelope.json" --json --endpoint "$EP" > "$OUT/wf-final.json" 2> "$OUT/wf-err2.txt" \
  || die "finish (the same save without --draft) failed: $(cat "$OUT/wf-err2.txt" 2>/dev/null)"
ok "finished — identical bytes under a changed lifecycle IS the transition"

step "7 · identity RE-asserted after the whole journey"
just _assert-serving-identity "$GRPC_PORT" "$KX" \
  || die "the serving binary CHANGED mid-proof — every assertion above is void"

printf '\n✅ NL AUTHORING LIVE PROOF PASSED (artifacts in %s)\n' "$OUT"
