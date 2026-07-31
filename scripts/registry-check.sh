#!/usr/bin/env bash
# Registry consistency: `registry/index.json` must agree with the tree.
#
# The failure this prevents is an ABSENCE — a capability-family artifact that merges
# green while missing from the index, or an index entry pointing at a directory nobody
# shipped. Neither shows up in any behavioural test, because the thing that is wrong is
# the thing that is not there.
#
#   1. index parses; schema tag; (family, name) unique; required fields non-empty
#   2. every entry's `source` path exists
#   3. skills/**                     <-> family=="skill"       entries (bidirectional)
#   4. integrations/kx-connector-*   <-> family=="integration" entries (bidirectional)
#   5. each skill entry's skill.json name/version match the index entry
#
# Deterministic local file reads only — no network, no cargo, well under a second.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import json, os, sys

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
REQUIRED = ("family", "name", "version", "source", "description", "conformance")
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

# 3. skills/** <-> skill entries (bidirectional: an orphan in EITHER direction is a bug)
tree_skills = set()
if os.path.isdir("skills"):
    tree_skills = {d for d in os.listdir("skills") if os.path.isdir(os.path.join("skills", d))}
index_skills = {e["name"] for e in entries if e.get("family") == "skill"}
for missing in sorted(tree_skills - index_skills):
    err(f"skills/{missing} exists in-tree but has no registry entry")
for ghost in sorted(index_skills - tree_skills):
    err(f"skill entry {ghost!r} has no skills/{ghost} directory")

# 4. integrations/kx-connector-* <-> integration entries (bidirectional)
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

if fail:
    print(f"registry-check: FAIL — {len(fail)} problem(s):")
    for f_ in fail:
        print(f"  {f_}")
    sys.exit(1)

print(f"registry-check: OK — {len(entries)} entr(y/ies), index agrees with the tree")
PY
