#!/usr/bin/env bash
# RC-SW1: the registry-consistency check (D175 test-infra enabler #3).
#
# Asserts registry/index.json agrees with the tree, so a capability-family
# artifact can never "merge green while missing from an index":
#   1. index parses; schema tag; (family, name) unique; all fields non-empty
#   2. every entry's `source` path exists
#   3. skills/** directories  ⟷  family=="skill" sources (bidirectional)
#   4. integrations/kx-connector-* workspace members ⟷ family=="integration" names
#   5. each skill entry's skill.json name/version match the index entry
#   6. every `ledger` id exists verbatim in feature-ledger.toml
#
# Deterministic local file reads only — no network, no cargo, <1s (CI-safe).
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import json, os, re, sys

fail = []

def err(msg):
    fail.append(msg)

# 1. parse + schema + uniqueness + required fields
try:
    with open("registry/index.json") as f:
        index = json.load(f)
except Exception as e:  # noqa: BLE001 - report, don't trace
    print(f"registry-check: FAIL — registry/index.json does not parse: {e}")
    sys.exit(1)

if index.get("schema") != "kortecx.registry/v1":
    err(f"schema must be kortecx.registry/v1, got {index.get('schema')!r}")

entries = index.get("entries", [])
REQUIRED = ("family", "name", "version", "source", "description", "conformance", "ledger")
FAMILIES = {"skill", "tool", "integration"}
seen = set()
for e in entries:
    key = (e.get("family"), e.get("name"))
    if key in seen:
        err(f"duplicate entry {key}")
    seen.add(key)
    for field in REQUIRED:
        if not e.get(field):
            err(f"{key}: empty/missing field {field!r}")
    if e.get("family") not in FAMILIES:
        err(f"{key}: unknown family {e.get('family')!r}")

# 2. sources exist
for e in entries:
    if e.get("source") and not os.path.exists(e["source"]):
        err(f"({e['family']}, {e['name']}): source path {e['source']!r} does not exist")

# 3. skills/** ⟷ skill entries (bidirectional)
tree_skills = set()
if os.path.isdir("skills"):
    tree_skills = {d for d in os.listdir("skills") if os.path.isdir(os.path.join("skills", d))}
index_skills = {e["name"] for e in entries if e.get("family") == "skill"}
for missing in sorted(tree_skills - index_skills):
    err(f"skills/{missing} exists in-tree but has no registry entry")
for ghost in sorted(index_skills - tree_skills):
    err(f"skill entry {ghost!r} has no skills/{ghost} directory")

# 4. integrations/kx-connector-* ⟷ integration entries (bidirectional)
tree_integrations = set()
if os.path.isdir("integrations"):
    tree_integrations = {
        d for d in os.listdir("integrations")
        if d.startswith("kx-connector-") and os.path.isdir(os.path.join("integrations", d))
    }
index_integrations = {e["name"] for e in entries if e.get("family") == "integration"}
for missing in sorted(tree_integrations - index_integrations):
    err(f"integrations/{missing} exists in-tree but has no registry entry")
for ghost in sorted(index_integrations - tree_integrations):
    err(f"integration entry {ghost!r} has no integrations/{ghost} directory")

# 5. skill manifests agree with their index entries
for e in entries:
    if e.get("family") != "skill" or not e.get("source"):
        continue
    manifest_path = os.path.join(e["source"], "skill.json")
    try:
        with open(manifest_path) as f:
            m = json.load(f)
    except Exception as exc:  # noqa: BLE001
        err(f"{manifest_path}: does not parse: {exc}")
        continue
    if m.get("name") != e["name"]:
        err(f"{manifest_path}: manifest name {m.get('name')!r} != index name {e['name']!r}")
    if m.get("version", "1") != e["version"]:
        err(f"{manifest_path}: manifest version {m.get('version')!r} != index version {e['version']!r}")

# 6. ledger ids are real (verbatim id match — never free-text prose matching)
with open("feature-ledger.toml") as f:
    ledger = f.read()
ledger_ids = set(re.findall(r'^id\s*=\s*"([^"]+)"', ledger, re.M))
for e in entries:
    if e.get("ledger") and e["ledger"] not in ledger_ids:
        err(f"({e['family']}, {e['name']}): ledger id {e['ledger']!r} not in feature-ledger.toml")

if fail:
    print("registry-check: FAIL")
    for msg in fail:
        print(f"  - {msg}")
    sys.exit(1)
print(f"registry-check: OK — {len(entries)} entries consistent with the tree")
PY
