"""D170 secrets + triggers SDK surface — pure unit tests (no server).

Mirrors test_toolscout.py: exercises the view ``from_proto`` mappers + the
friendly kind/auth enum mapping (incl. fail-closed ``ValueError`` on an unknown
string), with no live gateway.
"""

from __future__ import annotations

import pytest

from kortecx import (
    SecretName,
    SecretNamesPage,
    TriggersPage,
    TriggerView,
    trigger_auth_name,
    trigger_auth_to_proto,
    trigger_kind_name,
    trigger_kind_to_proto,
)
from kortecx.v1 import gateway_pb2 as g


def test_secret_name_from_proto_carries_name_and_timestamps():
    sn = SecretName.from_proto(
        g.SecretName(name="OPENAI_KEY", created_unix_ms=1000, updated_unix_ms=2000)
    )
    assert sn.name == "OPENAI_KEY"
    assert sn.created_unix_ms == 1000
    assert sn.updated_unix_ms == 2000


def test_secret_names_page_holds_rows_and_has_more():
    page = SecretNamesPage(
        names=[SecretName(name="A", created_unix_ms=0, updated_unix_ms=0)],
        has_more=True,
    )
    assert [n.name for n in page.names] == ["A"]
    assert page.has_more is True


def test_trigger_kind_to_proto_maps_every_arm():
    assert trigger_kind_to_proto("webhook") == g.TriggerKind.WEBHOOK
    assert trigger_kind_to_proto("cron") == g.TriggerKind.CRON
    assert trigger_kind_to_proto("grpc") == g.TriggerKind.GRPC


def test_trigger_auth_to_proto_maps_every_arm():
    assert trigger_auth_to_proto("none") == g.TriggerAuth.NONE
    assert trigger_auth_to_proto("hmac_sha256") == g.TriggerAuth.HMAC_SHA256
    assert trigger_auth_to_proto("bearer") == g.TriggerAuth.BEARER


def test_trigger_kind_and_auth_to_proto_reject_unknown():
    with pytest.raises(ValueError, match="kind must be one of"):
        trigger_kind_to_proto("nope")
    with pytest.raises(ValueError, match="auth must be one of"):
        trigger_auth_to_proto("nope")


def test_trigger_kind_and_auth_name_round_trip_and_unknown():
    assert trigger_kind_name(g.TriggerKind.WEBHOOK) == "webhook"
    assert trigger_auth_name(g.TriggerAuth.BEARER) == "bearer"
    # "unknown" absorbs UNSPECIFIED(0) + any future value — never a crash.
    assert trigger_kind_name(g.TriggerKind.TRIGGER_KIND_UNSPECIFIED) == "unknown"
    assert trigger_auth_name(999) == "unknown"


def test_trigger_view_from_proto_hex_encodes_id_and_names_enums():
    tv = TriggerView.from_proto(
        g.TriggerView(
            trigger_id=b"\xab" * 16,
            name="nightly",
            kind=g.TriggerKind.CRON,
            recipe_handle="kx/recipes/echo",
            auth=g.TriggerAuth.HMAC_SHA256,
            auth_secret_present=True,
            schedule_spec="*/5 * * * *",
            enabled=True,
            last_fire_unix_ms=42,
        )
    )
    assert tv.trigger_id == "ab" * 16  # lowercase hex
    assert tv.name == "nightly"
    assert tv.kind == "cron"
    assert tv.recipe_handle == "kx/recipes/echo"
    assert tv.auth == "hmac_sha256"
    assert tv.auth_secret_present is True
    assert tv.schedule_spec == "*/5 * * * *"
    assert tv.enabled is True
    assert tv.last_fire_unix_ms == 42


def test_triggers_page_holds_rows_and_has_more():
    page = TriggersPage(
        triggers=[
            TriggerView(
                trigger_id="00" * 16,
                name="t",
                kind="webhook",
                recipe_handle="r",
                app_handle="",
                auth="none",
                auth_secret_present=False,
                schedule_spec="",
                timezone="",
                enabled=False,
                require_approval=False,
                last_fire_unix_ms=0,
            )
        ],
        has_more=False,
    )
    assert [t.name for t in page.triggers] == ["t"]
    assert page.has_more is False
