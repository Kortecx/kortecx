"""Batch B mote-detail view — pure mapper tests, no server."""

from __future__ import annotations

from kortecx import MoteConfigItem, MoteDetail, effect_pattern_name, nd_class_name
from kortecx.v1 import gateway_pb2 as g


def test_mote_detail_from_proto_maps_every_field():
    d = g.MoteDetail(
        mote_id=b"\xa1" * 32,
        mote_def_hash=b"\xb2" * 32,
        def_found=True,
        step_kind="model",
        model_id="qwen3",
        prompt="say hi",
        prompt_truncated=False,
        config_subset=[
            g.MoteConfigEntry(key="temperature", value=b"0", truncated=False, full_len=1)
        ],
        tool_contract={"echo": "1"},
        logic_ref=b"\x07" * 32,
        nd_class=1,
        effect_pattern=1,
        critic_for=b"\x03" * 32,
        is_topology_shaper=False,
        schema_version=5,
    )
    detail = MoteDetail.from_proto(d)
    assert detail.mote_id == "a1" * 32
    assert detail.mote_def_hash == "b2" * 32
    assert detail.def_found is True
    assert detail.nd_class_name == "PURE"
    assert detail.effect_pattern_name == "IdempotentByConstruction"
    assert detail.critic_for == "03" * 32
    assert detail.tool_contract == {"echo": "1"}
    assert detail.to_dict() == {
        "mote_id": "a1" * 32,
        "mote_def_hash": "b2" * 32,
        "def_found": True,
        "step_kind": "model",
        "model_id": "qwen3",
        "prompt": "say hi",
        "prompt_truncated": False,
        "config_subset": [
            {"key": "temperature", "value_hex": "30", "truncated": False, "full_len": 1}
        ],
        "tool_contract": {"echo": "1"},
        "logic_ref": "07" * 32,
        "nd_class": "PURE",
        "effect_pattern": "IdempotentByConstruction",
        "critic_for": "03" * 32,
        "is_topology_shaper": False,
        "schema_version": 5,
    }


def test_mote_detail_honest_empty():
    d = g.MoteDetail(mote_id=b"\xa1" * 32, def_found=False)
    detail = MoteDetail.from_proto(d)
    assert detail.def_found is False
    assert detail.mote_def_hash == ""
    assert detail.critic_for is None
    assert detail.to_dict()["critic_for"] is None


def test_display_name_maps_cover_the_closed_vocabularies():
    assert nd_class_name(1) == "PURE"
    assert nd_class_name(2) == "READ_ONLY_NONDET"
    assert nd_class_name(3) == "WORLD_MUTATING"
    assert nd_class_name(0) == "UNKNOWN"
    assert effect_pattern_name(1) == "IdempotentByConstruction"
    assert effect_pattern_name(2) == "StageThenCommit"
    assert effect_pattern_name(3) == "ValidateThenCommit"
    assert effect_pattern_name(99) == "UNKNOWN"


def test_config_item_truncation_is_honest():
    e = g.MoteConfigEntry(key="blob", value=b"a" * 8, truncated=True, full_len=5000)
    item = MoteConfigItem.from_proto(e)
    assert item.truncated is True
    assert item.full_len == 5000
    assert item.to_dict()["value_hex"] == "61" * 8
