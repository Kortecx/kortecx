"""Contract / conformance tests against a real ``kx serve``.

The headline test proves **byte-parity**: the SDK's ``invoke(..., wait=True)`` and
the reference ``kx`` CLI, hitting the same gateway with the same recipe + args,
produce identical server-derived ids and result (SN-8 holds across both surfaces).
The rest exercise the projection/content/events flow and the edge cases a mature
agentic runtime must handle: ownership rejection, auth, timeout, distinct-args
isolation, idempotency, and the signature catalog.
"""

from __future__ import annotations

import json
import subprocess

import pytest

from kortecx import (
    AsyncKxClient,
    KxClient,
    KxInvalidArgument,
    KxNotFound,
    KxPermissionDenied,
    KxUnauthenticated,
    KxWaitTimeout,
)

# The demo recipe the gateway provisions (also defined in conftest for the
# fixtures). Inlined here so the module imports under any pytest invocation
# (bare `pytest` does not put the rootdir on sys.path, unlike `python -m pytest`).
ECHO_HANDLE = "kx/recipes/echo"

# Fields of the invoke --wait --json shape that are derived from content (and so
# are identical across any server, language, or process — the SN-8 guarantee).
# `instance_id` is the one exception: it is assigned per journal registration, so
# it agrees only WITHIN a single server (proven by the read-back test below).
_DETERMINISTIC = ("terminal_mote_id", "result_ref", "result_hex", "result_len", "state")


def _cli(kx_bin, endpoint, *argv):
    out = subprocess.run(
        [kx_bin, *argv, "--json", "--endpoint", endpoint],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(out.stdout)


# --- the headline: SDK ⇄ CLI conformance -------------------------------------


def test_invoke_wait_matches_cli(serve, kx_bin):
    """The SDK's invoke→wait result equals the reference CLI's, field-for-field,
    on every content-derived (server-derived, SN-8) field. Two fresh servers so
    each owns the single invocation of `{"topic":"hello"}` (one run instance per
    recipe, so the same server can't be invoked with identical args twice)."""
    s_sdk, s_cli = serve("--dev-allow-local"), serve("--dev-allow-local")
    with KxClient(s_sdk.endpoint) as kx:
        sdk = kx.invoke(ECHO_HANDLE, {"topic": "hello"}, wait=True)
    cli = _cli(
        kx_bin, s_cli.endpoint, "invoke", ECHO_HANDLE, "--args", '{"topic":"hello"}', "--wait"
    )
    assert sdk.ok
    assert set(sdk.to_dict()) == set(cli)  # identical shape
    for k in _DETERMINISTIC:
        assert sdk.to_dict()[k] == cli[k], f"field {k} differs SDK vs CLI"


def test_sdk_and_cli_read_the_same_committed_result(dev_server, kx_bin):
    """On ONE server, the SDK and CLI agree on the SAME committed data — including
    the server-assigned instance_id (full parity, no second invoke needed)."""
    with KxClient(dev_server.endpoint) as kx:
        result = kx.invoke(ECHO_HANDLE, {"topic": "readback"}, wait=True)
        proj = kx.get_projection(result.instance_id)
    cli_proj = _cli(kx_bin, dev_server.endpoint, "projection", "--instance", result.instance_id)
    assert proj.to_dict() == cli_proj  # identical projection (incl. instance_id)
    cli_content = _cli(
        kx_bin,
        dev_server.endpoint,
        "content",
        "--ref",
        result.result_ref,
        "--instance",
        result.instance_id,
    )
    assert cli_content["payload_hex"] == result.to_dict()["result_hex"]


def test_events_and_poll_wait_modes_agree(dev_server):
    """Both wait strategies observe the same already-committed Mote on one run."""
    with KxClient(dev_server.endpoint) as kx:
        run = kx.invoke(ECHO_HANDLE, {"topic": "evt"})  # one invoke, no wait
        polled = run.wait(mode="poll")
        evented = run.wait(mode="events")
    assert polled.to_dict() == evented.to_dict()


@pytest.mark.asyncio
async def test_async_invoke_and_wait_modes(dev_server):
    async with AsyncKxClient(dev_server.endpoint) as kx:
        run = await kx.invoke(ECHO_HANDLE, {"topic": "async"})  # AsyncRun
        polled = await run.wait(mode="poll")
        evented = await run.wait(mode="events")
    assert polled.ok and polled.to_dict() == evented.to_dict()


def test_idempotent_reinvoke_same_args(dev_server):
    """Idempotent re-invoke: the same recipe+args resolves to the same already-
    committed terminal Mote and result (exactly-once-per-input). Regression for the
    coordinator duplicate-submit fix — previously the second invoke failed with
    `UNAVAILABLE: non-16-byte instance_id` because the duplicate path dropped the
    run's instance_id."""
    with KxClient(dev_server.endpoint) as kx:
        a = kx.invoke(ECHO_HANDLE, {"topic": "same"}, wait=True)
        b = kx.invoke(ECHO_HANDLE, {"topic": "same"}, wait=True)
    assert a.instance_id == b.instance_id
    assert a.terminal_mote_id == b.terminal_mote_id
    assert a.result_ref == b.result_ref


# --- the projection → content flow -------------------------------------------


def test_run_handle_projection_and_content(dev_server):
    with KxClient(dev_server.endpoint) as kx:
        run = kx.invoke(ECHO_HANDLE, {"topic": "flow"})  # no wait → a handle
        result = run.wait(timeout=30)
        assert result.ok
        proj = run.projection()
        assert proj.instance_id == run.instance_id
        terminal = proj.mote(run.terminal_mote_id)
        assert terminal is not None and terminal.state == "COMMITTED"
        # Fetch the committed content by its ref; bytes equal the wait payload.
        payload = run.content(terminal.result_ref)
        assert payload == result.bytes


def test_stream_events_snapshot_sees_terminal_commit(dev_server):
    with KxClient(dev_server.endpoint) as kx:
        run = kx.invoke(ECHO_HANDLE, {"topic": "events"})
        run.wait(timeout=30)  # ensure it has committed
        deltas = list(run.events(since=0, follow=False))
    committed = [d for d in deltas if d.kind == "committed"]
    assert any(d.mote_id == run.terminal_mote_id for d in committed)


def test_ws_events_bridge_sees_terminal_commit(dev_server):
    """The optional WebSocket live-tail client consumes the same deltas (R5 bridge)."""
    pytest.importorskip("websockets")
    with KxClient(dev_server.endpoint) as kx:
        run = kx.invoke(ECHO_HANDLE, {"topic": "ws"}, wait=True)  # commit before subscribing
        found = False
        for i, d in enumerate(
            kx.ws_events(run.instance_id, since=0, ws_endpoint=dev_server.ws_endpoint)
        ):
            if d.kind == "committed" and d.mote_id == run.terminal_mote_id:
                found = True
                break
            if i > 200:  # safety: the catch-up replay carries it in the first frame
                break
    assert found


# --- isolation + idempotency --------------------------------------------------


def test_distinct_args_distinct_terminal_same_instance(dev_server):
    with KxClient(dev_server.endpoint) as kx:
        a = kx.invoke(ECHO_HANDLE, {"topic": "alpha"}, wait=True)
        b = kx.invoke(ECHO_HANDLE, {"topic": "beta"}, wait=True)
    assert a.instance_id == b.instance_id  # one run instance per recipe
    assert a.terminal_mote_id != b.terminal_mote_id  # distinct Mote per input


def test_determinism_across_fresh_servers(serve):
    """The same recipe + args on two fresh servers yields the SAME content-derived
    ids (terminal Mote, result ref, result bytes) — SN-8 determinism."""
    s1, s2 = serve("--dev-allow-local"), serve("--dev-allow-local")
    with KxClient(s1.endpoint) as a, KxClient(s2.endpoint) as b:
        ra = a.invoke(ECHO_HANDLE, {"topic": "det"}, wait=True)
        rb = b.invoke(ECHO_HANDLE, {"topic": "det"}, wait=True)
    assert ra.terminal_mote_id == rb.terminal_mote_id
    assert ra.result_ref == rb.result_ref and ra.bytes == rb.bytes


# --- edge cases a mature runtime must hold -----------------------------------


def test_ownership_rejection_is_uniform_permission_denied(dev_server):
    with KxClient(dev_server.endpoint) as kx:
        run = kx.invoke(ECHO_HANDLE, {"topic": "owned"}, wait=True)
        bogus_instance = "00" * 16
        with pytest.raises(KxPermissionDenied):
            kx.get_projection(bogus_instance)
        with pytest.raises(KxPermissionDenied):
            # right ref, wrong ownership ticket → uniform permission denied
            kx.get_content(run.result_ref, bogus_instance)


def test_wait_timeout_is_resumable(dev_server):
    with KxClient(dev_server.endpoint) as kx:
        run = kx.invoke(ECHO_HANDLE, {"topic": "timeout"})  # real instance
        # Wait on a Mote id that will never appear in this run → times out fast.
        with pytest.raises(KxWaitTimeout) as ei:
            kx._await_terminal(run.instance_id_bytes, b"\x00" * 32, 0.6, "poll")
        assert ei.value.instance_id == run.instance_id  # resumable handle carried


def test_unauthenticated_without_token(auth_server):
    with KxClient(auth_server.endpoint) as kx:  # no token
        with pytest.raises(KxUnauthenticated):
            kx.invoke(ECHO_HANDLE, {"topic": "x"}, wait=True)


def test_authenticated_with_token(auth_server):
    with KxClient(auth_server.endpoint, token=auth_server.token) as kx:
        result = kx.invoke(ECHO_HANDLE, {"topic": "authed"}, wait=True)
    assert result.ok


# --- signature catalog --------------------------------------------------------


def test_signatures_list_empty_and_unknown_not_found(dev_server):
    with KxClient(dev_server.endpoint) as kx:
        assert kx.list_signatures() == []
        with pytest.raises(KxNotFound):
            kx.get_signature("00" * 32)
        with pytest.raises(KxInvalidArgument):
            kx.register_signature(b"not a valid signature manifest")


def test_submit_run_low_level_validates(dev_server):
    """The low-level submit_run passthrough surfaces server validation as a typed
    error (a short recipe_fingerprint is INVALID_ARGUMENT)."""
    from kortecx.v1 import gateway_pb2 as g

    with KxClient(dev_server.endpoint) as kx:
        with pytest.raises(KxInvalidArgument):
            kx.submit_run(g.SubmitRunRequest(recipe_fingerprint=b"short"))
