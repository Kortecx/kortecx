"""UI-2 run-summary + recipe-form + react/replan views — pure unit tests."""

from __future__ import annotations

from kortecx import (
    CaptureRecord,
    ReactTurn,
    RecipeForm,
    RecipeFormField,
    RecipeInfo,
    ReplanRound,
    RunInputs,
    RunSummary,
    ScoredRecipe,
    recipe_param_type_name,
)
from kortecx.v1 import gateway_pb2 as g


def test_recipe_param_type_name_maps_all_and_absorbs_unknown():
    assert recipe_param_type_name(g.RECIPE_PARAM_TYPE_STR) == "str"
    assert recipe_param_type_name(g.RECIPE_PARAM_TYPE_INT) == "int"
    assert recipe_param_type_name(g.RECIPE_PARAM_TYPE_BOOL) == "bool"
    assert recipe_param_type_name(g.RECIPE_PARAM_TYPE_BYTES) == "bytes"
    assert recipe_param_type_name(g.RECIPE_PARAM_TYPE_ENUM) == "enum"
    assert recipe_param_type_name(g.RECIPE_PARAM_TYPE_UNSPECIFIED) == "unspecified"
    assert recipe_param_type_name(99) == "unspecified"


def test_run_summary_from_proto_hex_encodes_and_carries_seq_ts():
    r = g.RunSummary(
        instance_id=b"\x11" * 16,
        recipe_fingerprint=b"\x22" * 32,
        registered_seq=7,
        registered_unix_ms=1234,
    )
    s = RunSummary.from_proto(r)
    assert s.instance_id == "11" * 16
    assert s.recipe_fingerprint == "22" * 32
    assert s.registered_seq == 7
    assert s.registered_unix_ms == 1234


def test_run_inputs_from_proto_decodes_args_and_handle():
    r = g.GetRunInputsResponse(
        instance_id=b"\x11" * 16,
        recipe_fingerprint=b"\x22" * 32,
        handle="kx/recipes/echo",
        args=b'{"topic":"hi","count":3}',
    )
    ri = RunInputs.from_proto(r)
    assert ri.instance_id == "11" * 16
    assert ri.recipe_fingerprint == "22" * 32
    assert ri.handle == "kx/recipes/echo"
    assert ri.args == {"topic": "hi", "count": 3}


def test_run_inputs_empty_non_object_or_malformed_args_become_empty_dict():
    empty = RunInputs.from_proto(g.GetRunInputsResponse(handle="h"))
    assert empty.args == {}
    arr = RunInputs.from_proto(g.GetRunInputsResponse(handle="h", args=b"[1,2,3]"))
    assert arr.args == {}
    # A corrupt/non-JSON capture degrades to {} rather than throwing.
    bad = RunInputs.from_proto(g.GetRunInputsResponse(handle="h", args=b"not json{"))
    assert bad.args == {}


def test_recipe_form_field_typed_str_with_max_len():
    f = g.RecipeFormField(name="topic", type=g.RECIPE_PARAM_TYPE_STR, required=True, max_len=4096)
    field = RecipeFormField.from_proto(f)
    assert field.name == "topic"
    assert field.type == "str"
    assert field.required is True
    assert field.max_len == 4096
    assert field.allowed == []


def test_recipe_form_field_enum_has_allowed_and_no_max_len():
    f = g.RecipeFormField(
        name="mode", type=g.RECIPE_PARAM_TYPE_ENUM, required=True, allowed=["fast", "slow"]
    )
    field = RecipeFormField.from_proto(f)
    assert field.type == "enum"
    assert field.max_len is None
    assert field.allowed == ["fast", "slow"]


def test_react_turn_from_proto_hex_encodes_branch_tool_caps():
    t = g.ReactTurnSummary(
        turn=2,
        turn_mote_id=b"\x28" * 32,
        instance_id=b"\x05" * 16,
        model_id="react-v1",
        branch="tool",
        tool_id="mcp-echo",
        tool_version="1",
        max_turns=8,
        max_tool_calls=6,
        seq=42,
    )
    r = ReactTurn.from_proto(t)
    assert r.turn == 2
    assert r.turn_mote_id == "28" * 32
    assert r.instance_id == "05" * 16
    assert r.branch == "tool"
    assert r.tool_id == "mcp-echo"
    assert r.max_tool_calls == 6
    assert r.seq == 42


def test_react_turn_answer_branch_has_empty_tool_fields():
    t = g.ReactTurnSummary(
        turn=0,
        turn_mote_id=b"\x01" * 32,
        instance_id=b"\x02" * 16,
        model_id="m",
        branch="answer",
        max_turns=8,
        max_tool_calls=6,
        seq=3,
    )
    r = ReactTurn.from_proto(t)
    assert r.branch == "answer"
    assert r.tool_id == ""
    assert r.tool_version == ""


def test_replan_round_from_proto_hex_encodes_shaper_and_failed_steps():
    rr = g.ReplanRoundSummary(
        round=1,
        shaper_mote_id=b"\x1e" * 32,
        model_id="plan-v1",
        failed_step_ids=[b"\x1f" * 32, b"\x20" * 32],
        escalated=False,
        seq=9,
    )
    round_ = ReplanRound.from_proto(rr)
    assert round_.round == 1
    assert round_.shaper_mote_id == "1e" * 32
    assert round_.failed_step_ids == ["1f" * 32, "20" * 32]
    assert round_.escalated is False
    assert round_.seq == 9


def test_recipe_form_wraps_handle_and_fields():
    resp = g.GetRecipeFormResponse(
        handle="kx/recipes/echo",
        fields=[
            g.RecipeFormField(
                name="topic", type=g.RECIPE_PARAM_TYPE_STR, required=True, max_len=4096
            )
        ],
    )
    form = RecipeForm.from_proto(resp)
    assert form.handle == "kx/recipes/echo"
    assert len(form.fields) == 1
    assert form.fields[0].name == "topic"


def test_recipe_info_from_proto_carries_advisory_metadata():
    # PR-4 Batch D: ListRecipes now carries description / tags / version.
    s = g.RecipeSummary(
        handle="kx/recipes/echo",
        recipe_fingerprint=b"\xab" * 32,
        description="Echo — passthrough",
        tags=["passthrough", "pure"],
        version="abc123def456",
    )
    ri = RecipeInfo.from_proto(s)
    assert ri.handle == "kx/recipes/echo"
    assert ri.recipe_fingerprint == "ab" * 32
    assert ri.description == "Echo — passthrough"
    assert ri.tags == ["passthrough", "pure"]
    assert ri.version == "abc123def456"


def test_scored_recipe_from_proto_maps_score_and_recipe():
    # SN-8: score_bp is an integer (basis points), never a float.
    sr = g.ScoredRecipe(
        recipe=g.RecipeSummary(handle="kx/recipes/chat", description="Chat"),
        score_bp=7000,
    )
    out = ScoredRecipe.from_proto(sr)
    assert out.score_bp == 7000
    assert isinstance(out.score_bp, int)
    assert out.recipe.handle == "kx/recipes/chat"
    assert out.recipe.description == "Chat"


def test_recipe_form_empty_is_valid():
    resp = g.GetRecipeFormResponse(handle="kx/recipes/passthrough-dag")
    form = RecipeForm.from_proto(resp)
    assert form.fields == []


def test_capture_record_from_proto_hex_encodes_and_react_join():
    r = g.CaptureRecordSummary(
        mote_id=b"\x28" * 32,
        instance_id=b"\x05" * 16,
        result_ref=b"\x30" * 32,
        nd_class="read_only_nondet",
        seq=7,
        react_turn=2,
        react_branch="tool",
    )
    rec = CaptureRecord.from_proto(r)
    assert rec.mote_id == "28" * 32
    assert rec.instance_id == "05" * 16
    assert rec.result_ref == "30" * 32
    assert rec.nd_class == "read_only_nondet"
    assert rec.seq == 7
    assert rec.react_turn == 2
    assert rec.react_branch == "tool"


def test_capture_record_non_react_action_has_none_turn():
    r = g.CaptureRecordSummary(
        mote_id=b"\x01" * 32,
        instance_id=b"\x02" * 16,
        result_ref=b"\x03" * 32,
        nd_class="pure",
        seq=1,
    )
    rec = CaptureRecord.from_proto(r)
    assert rec.react_turn is None
    assert rec.react_branch == ""
