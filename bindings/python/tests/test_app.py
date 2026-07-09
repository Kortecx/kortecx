"""POC-4 App-authoring SDK tests.

- **builder**: ``app().blueprint(flow()...).to_envelope()`` produces the canonical
  ``kortecx.app/v1`` shape; pending bodies are rejected by ``to_envelope`` (use
  ``save``); a referenced body never inlines into the envelope (secret-leak).
- **golden parity** (the cross-surface gate): every committed canonical envelope in
  ``tests/golden/apps/corpus.json`` round-trips through this SDK's canonicalizer
  byte-identically (matches the Rust ``kx-app`` + the TS SDK).
- **server-backed** (a real ``kx serve``): ``save_app`` → ``list_apps`` → ``get_app``
  round-trips the envelope; ``run_app`` compiles the blueprint and runs it.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import kortecx as kx
from kortecx import Skill
from kortecx.apps import canonical_json
from kortecx.chains import ChainError
from kortecx.client import KxClient

_CORPUS_PATH = Path(__file__).resolve().parents[3] / "tests" / "golden" / "apps" / "corpus.json"
_CORPUS = json.loads(_CORPUS_PATH.read_text())


# ---- builder (no server) ----


def test_builder_to_envelope_shape() -> None:
    a = (
        kx.app("Echo Demo")
        .blueprint(kx.flow().agent("Use the echo tool.", tools=["mcp-echo/echo"]))
        .steer(max_turns=8, max_tool_calls=6)
        .tags("demo")
        .describe("fires echo")
    )
    env = a.to_envelope()
    assert env["schema"] == "kortecx.app/v1"
    assert env["name"] == "Echo Demo"
    assert env["version"] == "1"
    assert env["blueprint"]["steps"][0]["tool_contract"] == {"mcp-echo/echo": "1"}
    assert env["steering_config"]["guards"] == {"max_turns": 8, "max_tool_calls": 6}
    assert env["tags"] == ["demo"]
    # empty rails are omitted (the canonical omit-empty discipline).
    assert "references" not in env
    assert "replay" not in env
    # the envelope canonicalizes + round-trips.
    canon = canonical_json(env)
    assert json.loads(canon.decode()) == env


def test_use_tool_dual_writes_the_wish_and_the_display_rail() -> None:
    # use_tool records BOTH the display ref (references.tools) AND the wish the server
    # actually consumes (steering_config.tools.requested_grants).
    env = (
        kx.app("x")
        .blueprint(kx.flow().agent("go"))
        .use_tool("mcp-echo/echo")
        .use_tool("retrieve", "2")
        .to_envelope()
    )
    assert env["references"]["tools"] == [
        {"tool_id": "mcp-echo/echo", "tool_version": "1"},
        {"tool_id": "retrieve", "tool_version": "2"},
    ]
    assert env["steering_config"]["tools"]["requested_grants"] == {
        "mcp-echo/echo": "1",
        "retrieve": "2",
    }


def test_reach_default_is_omitted_and_inherit_emits() -> None:
    # Default reach ⇒ no reach key (byte-identical to the pre-reach form).
    env = kx.app("x").blueprint(kx.flow().agent("go")).steer(requested_grants={"e": "1"}).to_envelope()
    assert "reach" not in env["steering_config"]["tools"]
    # inherit_principal ⇒ emitted, alongside any wish.
    env2 = (
        kx.app("x")
        .blueprint(kx.flow().agent("go"))
        .steer(reach=kx.REACH_INHERIT_PRINCIPAL)
        .to_envelope()
    )
    assert env2["steering_config"]["tools"] == {"reach": "inherit_principal"}


def test_reach_invalid_value_raises() -> None:
    with pytest.raises(ValueError):
        kx.app("x").blueprint(kx.flow().agent("go")).steer(reach="everything")


def test_blueprint_required() -> None:
    with pytest.raises(ChainError):
        kx.app("x").to_envelope()


def test_minimal_app_envelope_poc5a() -> None:
    env = kx.minimal_app_envelope("PDF Summarizer", "Summarize uploaded PDFs", model="gemma-4")
    assert env["schema"] == "kortecx.app/v1"
    assert env["name"] == "PDF Summarizer"
    assert env["description"] == "Summarize uploaded PDFs"
    assert env["steering_config"]["model"] == {"model_route": "gemma-4"}
    # a non-empty blueprint (a single agentic step) + canonical round-trip.
    assert env["blueprint"]
    canon = canonical_json(env)
    assert json.loads(canon.decode()) == env


def test_pending_body_blocks_to_envelope() -> None:
    a = kx.app("x").blueprint(kx.flow().step(topic="hi")).rule("no-pii", body="secret-body")
    with pytest.raises(ChainError):
        a.to_envelope()  # an unresolved body upload — use save()


def test_by_ref_artifact_never_inlines_a_body() -> None:
    # A rule referenced by content_ref carries ONLY the ref, never the bytes.
    ref = "a" * 64
    a = kx.app("x").blueprint(kx.flow().step(topic="hi")).rule("policy", ref=ref)
    canon = canonical_json(a.to_envelope()).decode()
    assert ref in canon
    assert "secret" not in canon


def test_skill_by_ref() -> None:
    a = (
        kx.app("x")
        .blueprint(kx.flow().step(topic="hi"))
        .skill(Skill(name="researcher", instructions_ref="b" * 64, tools={"mcp-echo/echo": "1"}))
    )
    env = a.to_envelope()
    assert env["references"]["skills"][0]["tools"] == {"mcp-echo/echo": "1"}


def test_dataset_grounding_by_ref() -> None:
    # RAG-on-App (T-RUNAPP-CONTEXT-RAIL): .dataset()/.rag() populate references.datasets.
    a = (
        kx.app("analyst")
        .blueprint(kx.flow().agent("Answer grounded."))
        .dataset("research")
        .rag("archive", cas_refs=["c" * 64])
    )
    datasets = a.to_envelope()["references"]["datasets"]
    assert datasets[0] == {"dataset_ref": "research"}  # no cas_refs ⇒ omitted
    assert datasets[1] == {"dataset_ref": "archive", "cas_refs": ["c" * 64]}


def test_dataset_rejects_non_hex_cas_ref() -> None:
    with pytest.raises(ChainError):
        kx.app("x").blueprint(kx.flow().step(topic="hi")).dataset("d", cas_refs=["not-hex"])


# ---- golden corpus parity (the cross-surface byte-shape gate) ----


@pytest.mark.parametrize("case", _CORPUS, ids=[c["name"] for c in _CORPUS])
def test_golden_corpus_round_trips_byte_identically(case) -> None:
    s = case["canonical"]
    parsed = json.loads(s)
    assert canonical_json(parsed).decode() == s, case["name"]


def test_corpus_covers_required_shapes() -> None:
    names = {c["name"] for c in _CORPUS}
    assert {"minimal", "agentic", "full", "grounded", "reach"} <= names


# ---- server-backed (a real kx serve) ----


def test_save_list_get_run_round_trip(dev_server) -> None:
    with KxClient(dev_server.endpoint) as client:
        # A model-free PURE blueprint so the run reaches Committed without a model.
        a = kx.app("Pure Demo").blueprint(kx.flow().step(topic="kortecx")).describe("pure")
        saved = a.save(client=client)
        assert not saved.deduplicated
        assert saved.handle == "apps/local/pure-demo"

        apps = client.list_apps()
        assert any(s.handle == "apps/local/pure-demo" and s.name == "Pure Demo" for s in apps)

        stored = client.get_app("apps/local/pure-demo")
        assert stored is not None
        assert stored.envelope["name"] == "Pure Demo"
        assert stored.summary.step_count == 1

        # The handle-free portable identity is surfaced: 64 hex chars (32 bytes),
        # stable across an identical re-fetch, and wider than the handle-scoped app_ref.
        assert len(stored.app_digest) == 64
        assert all(c in "0123456789abcdef" for c in stored.app_digest)
        assert client.get_app("apps/local/pure-demo").app_digest == stored.app_digest
        assert stored.app_digest != stored.summary.app_ref

        # identical re-save dedups (content-addressed).
        again = a.save(client=client)
        assert again.deduplicated

        # run_app compiles the blueprint and runs it (model-free pure step commits).
        result = client.run_app("apps/local/pure-demo", wait=True, timeout=60.0)
        assert result is not None


def test_get_missing_is_none(dev_server) -> None:
    with KxClient(dev_server.endpoint) as client:
        assert client.get_app("apps/local/nope") is None


def test_save_uploads_pending_body(dev_server) -> None:
    with KxClient(dev_server.endpoint) as client:
        a = (
            kx.app("With Rule")
            .blueprint(kx.flow().step(topic="hi"))
            .rule("no-pii", body="Never reveal personal data.")
        )
        a.save(client=client)
        stored = client.get_app("apps/local/with-rule")
        assert stored is not None
        rules = stored.envelope["references"]["rules"]
        # the body was uploaded → a 64-hex content_ref; the bytes never inlined.
        assert len(rules[0]["content_ref"]) == 64
        assert "Never reveal" not in json.dumps(stored.envelope)


def test_inject_app_args_is_pure_and_no_op_when_empty() -> None:
    """POC-5d: run_app(args=...) folds inputs into the entry model step's prompt;
    a no-op (same object) when empty, and it never mutates the source blueprint."""
    from kortecx.client import _inject_app_args

    bp = {
        "seed": 0,
        "steps": [
            {"kind": "pure"},
            {"kind": "model", "model_id": "m", "prompt": "Answer the question."},
        ],
    }
    # Empty/absent ⇒ byte-identical (same object).
    assert _inject_app_args(bp, None) is bp
    assert _inject_app_args(bp, {}) is bp

    # Folds into the FIRST model step, leaves the pure step untouched, no mutation.
    out = _inject_app_args(bp, {"topic": "kortecx", "n": "3"})
    assert out is not bp
    assert out["steps"][0] == {"kind": "pure"}
    model_prompt = out["steps"][1]["prompt"]
    assert "Answer the question." in model_prompt
    assert "Inputs:" in model_prompt
    assert "- topic: kortecx" in model_prompt
    assert "- n: 3" in model_prompt
    assert bp["steps"][1]["prompt"] == "Answer the question."  # source unchanged

    # No model step ⇒ unchanged.
    tool_only = {"seed": 0, "steps": [{"kind": "tool", "tool_contract": {"x/y": "1"}}]}
    assert _inject_app_args(tool_only, {"a": "b"}) is tool_only
