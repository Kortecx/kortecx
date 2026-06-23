"""POC-1 ``ServerInfo`` view + ``chat`` arg-construction — pure unit tests (no server).

``ServerInfo.from_proto`` projects the ``GetServerInfoResponse`` fields (display/
settings-only, never a secret — SN-8). ``KxClient.chat`` is a thin convenience over
``invoke(wait=True)``: it picks the plain ``kx/recipes/chat`` recipe — or the AUTO-RAG
``kx/recipes/chat-rag`` when a ``dataset`` is given — and returns the answer text. The
tests stub the client's ``invoke`` to assert the handle + args + decoded answer
without a running gateway.
"""

from __future__ import annotations

from kortecx import ServerInfo
from kortecx.client import (
    CHAT_RAG_RECIPE_HANDLE,
    CHAT_RECIPE_HANDLE,
    KxClient,
)
from kortecx.run import Result
from kortecx.v1 import gateway_pb2 as g


def test_server_info_from_proto_projects_every_field():
    r = g.GetServerInfoResponse(
        model_id="gemma-3-12b",
        model_path="/models/gemma.gguf",
        listen_addr="127.0.0.1:50151",
        ws_addr="127.0.0.1:50152",
        console_addr="127.0.0.1:50153",
        metrics_addr="127.0.0.1:9090",
        content_root="/var/kortecx/content",
        journal_path="/var/kortecx/journal.db",
        catalog_dir="/var/kortecx/recipes",
        max_lease=64,
        content_max_bytes=33554432,
        cors_origins=["https://app.example.com", "http://localhost:5173"],
        tls_enabled=True,
        auth_mode="token",
        feature_hnsw=True,
        feature_inference=True,
        feature_console=False,
        feature_vision=False,
        audit_log_enabled=True,
    )
    info = ServerInfo.from_proto(r)
    assert info.model_id == "gemma-3-12b"
    assert info.model_path == "/models/gemma.gguf"
    assert info.listen_addr == "127.0.0.1:50151"
    assert info.ws_addr == "127.0.0.1:50152"
    assert info.console_addr == "127.0.0.1:50153"
    assert info.metrics_addr == "127.0.0.1:9090"
    assert info.content_root == "/var/kortecx/content"
    assert info.journal_path == "/var/kortecx/journal.db"
    assert info.catalog_dir == "/var/kortecx/recipes"
    assert info.max_lease == 64
    assert info.content_max_bytes == 33554432
    assert info.cors_origins == ("https://app.example.com", "http://localhost:5173")
    assert info.tls_enabled is True
    assert info.auth_mode == "token"
    assert info.feature_hnsw is True
    assert info.feature_inference is True
    assert info.feature_console is False
    assert info.feature_vision is False
    assert info.audit_log_enabled is True


def test_server_info_defaults_are_honest_empties():
    # A minimal FFI-free / no-bridge gateway answers with empties + false flags
    # (honest, never fabricated) — never a secret either way (SN-8).
    info = ServerInfo.from_proto(g.GetServerInfoResponse())
    assert info.model_id == ""
    assert info.ws_addr == "" and info.console_addr == "" and info.metrics_addr == ""
    assert info.cors_origins == ()
    assert info.max_lease == 0 and info.content_max_bytes == 0
    assert not info.tls_enabled
    assert not info.feature_hnsw and not info.feature_inference
    assert not info.feature_console and not info.feature_vision
    assert not info.audit_log_enabled


def test_server_info_is_frozen():
    info = ServerInfo.from_proto(g.GetServerInfoResponse(model_id="m"))
    try:
        info.model_id = "tampered"  # type: ignore[misc]
    except Exception as e:  # frozen dataclass → FrozenInstanceError (an AttributeError)
        assert "model_id" in str(e) or isinstance(e, AttributeError)
    else:
        raise AssertionError("ServerInfo must be frozen (server-derived, display-only)")


class _FakeInvokeClient:
    """Records the (handle, args, wait, timeout) of the single chat invoke and
    returns a canned committed :class:`Result` — no gateway needed."""

    def __init__(self, payload: bytes = b"the answer") -> None:
        self.payload = payload
        self.calls: list[dict] = []

    def invoke(self, handle, args, *, wait=False, timeout=120.0):  # noqa: ANN001
        self.calls.append({"handle": handle, "args": args, "wait": wait, "timeout": timeout})
        return Result(
            instance_id="ab" * 8,
            terminal_mote_id="cd" * 32,
            state="COMMITTED",
            result_ref="ef" * 32,
            payload=self.payload,
        )


def test_chat_plain_uses_chat_recipe_and_returns_text():
    fc = _FakeInvokeClient(payload=b"hello there")
    answer = KxClient.chat(fc, "hi")  # type: ignore[arg-type]  # duck-typed fake
    assert answer == "hello there"
    call = fc.calls[0]
    assert call["handle"] == CHAT_RECIPE_HANDLE
    assert call["args"] == {"prompt": "hi"}
    assert call["wait"] is True


def test_chat_with_dataset_uses_rag_recipe_and_carries_k():
    fc = _FakeInvokeClient(payload=b"grounded answer")
    answer = KxClient.chat(fc, "what is X?", dataset="corpus", k=7)  # type: ignore[arg-type]
    assert answer == "grounded answer"
    call = fc.calls[0]
    assert call["handle"] == CHAT_RAG_RECIPE_HANDLE
    assert call["args"] == {"prompt": "what is X?", "dataset": "corpus", "k": 7}


def test_chat_default_k_is_four():
    fc = _FakeInvokeClient()
    KxClient.chat(fc, "q", dataset="ds")  # type: ignore[arg-type]
    assert fc.calls[0]["args"]["k"] == 4


def test_chat_empty_dataset_string_still_grounds():
    # An empty-string dataset is explicit (not None) → RAG recipe; the SERVER then
    # degrades honestly to a plain answer (the SDK never decides grounding).
    fc = _FakeInvokeClient()
    KxClient.chat(fc, "q", dataset="")  # type: ignore[arg-type]
    assert fc.calls[0]["handle"] == CHAT_RAG_RECIPE_HANDLE


def test_chat_returns_empty_string_when_no_text_payload():
    # A non-UTF-8 / absent payload yields "" (Result.text is None) — chat is typed
    # `-> str`, so it never leaks a None.
    fc = _FakeInvokeClient(payload=b"\xff\xfe")
    assert KxClient.chat(fc, "q") == ""  # type: ignore[arg-type]
