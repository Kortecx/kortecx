"""UI-2 run-summary + recipe-form views — pure unit tests (no server)."""

from __future__ import annotations

from kortecx import RecipeForm, RecipeFormField, RunSummary, recipe_param_type_name
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


def test_recipe_form_empty_is_valid():
    resp = g.GetRecipeFormResponse(handle="kx/recipes/fanout-demo")
    form = RecipeForm.from_proto(resp)
    assert form.fields == []
