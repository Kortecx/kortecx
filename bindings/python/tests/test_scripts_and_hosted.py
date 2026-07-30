"""The script registry and hosted Apps, from Python.

Two different kinds of test live here, deliberately.

The script tests are PARITY tests: they register through the SDK and read back
through the ``kx`` CLI as a subprocess, then assert the SERVER-DERIVED fields
agree. That is the interesting claim — two languages, two processes, one
identity. Asserting the SDK agrees with itself would prove nothing.

The hosted-app tests assert a REFUSAL. The conftest serve is a default-feature
build, so every hosted RPC answers ``UNIMPLEMENTED``. That is a real contract
worth pinning: the SDK must surface it as ``KxUnimplemented`` rather than as an
empty list, because an empty list reads as "you have no hosted apps" and is
indistinguishable from a serve that cannot host at all.
"""

from __future__ import annotations

import json
import subprocess

import pytest

from kortecx import KxClient
from kortecx.errors import KxUnimplemented
from kortecx.hosted_apps import HostedAppStatus

_SOURCE = b'#!/bin/sh\nread -r line\necho "seen:$line"\n'


def _kx(kx_bin: str, endpoint: str, *args: str) -> str:
    out = subprocess.run(
        [str(kx_bin), *args, "--endpoint", endpoint, "--json"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert out.returncode == 0, f"kx {' '.join(args)} failed: {out.stderr}"
    return out.stdout


def test_registered_script_has_the_same_identity_from_both_surfaces(dev_server, kx_bin):
    """The SDK registers; the CLI reads back. Every server-derived field agrees."""
    with KxClient(dev_server.endpoint) as kx_client:
        script_id = kx_client.register_script(
            name="parity-echo",
            version="1",
            interpreter="sh",
            source=_SOURCE,
            description="registered by the python sdk",
        )
        assert len(script_id) == 32, "script_id is a 16-byte server-derived id in hex"

        # Read the SAME row back through the SDK...
        got = kx_client.get_script("parity-echo", "1")
        assert got is not None
        assert got.script is not None
        assert got.source == _SOURCE, "the exact bytes that run come back verbatim"
        assert got.script.script_id == script_id

        # ...and through the CLI, a different language in a different process.
        doc = json.loads(_kx(kx_bin, dev_server.endpoint, "scripts", "get", "parity-echo", "1"))
        assert doc["script_id"] == script_id, "one identity across both surfaces"
        assert doc["source_ref"] == got.script.source_ref, "the content ref agrees"
        assert doc["interpreter"] == "sh"


def test_list_and_deregister_round_trip(dev_server):
    with KxClient(dev_server.endpoint) as kx_client:
        kx_client.register_script(name="parity-tidy", version="2", interpreter="sh", source=_SOURCE)
        page = kx_client.list_scripts(limit=64)
        names = {(s.script_name, s.script_version) for s in page.scripts}
        assert ("parity-tidy", "2") in names

        assert kx_client.deregister_script("parity-tidy", "2") is True
        assert kx_client.get_script("parity-tidy", "2") is None
        # A second deregister is not an error — it is simply nothing to do.
        assert kx_client.deregister_script("parity-tidy", "2") is False


def test_a_missing_script_is_a_uniform_none(dev_server):
    """Absent and not-owned are indistinguishable, by design."""
    with KxClient(dev_server.endpoint) as kx_client:
        assert kx_client.get_script("never-registered", "1") is None


def test_hosted_apps_refuse_loudly_without_the_feature(dev_server):
    """A default-feature serve cannot host, and says so.

    The failure mode this pins is an SDK that returns ``[]`` here: the caller
    cannot tell "no hosted apps" from "this serve will never host anything".
    """
    with KxClient(dev_server.endpoint) as kx_client:
        with pytest.raises(KxUnimplemented):
            kx_client.list_hosted_apps()
        with pytest.raises(KxUnimplemented):
            kx_client.get_hosted_app_status("acme/demo/site")
        with pytest.raises(KxUnimplemented):
            kx_client.start_hosted_app("acme/demo/site")


def test_hosted_app_status_reads_an_absent_serve_mode_as_dev():
    """An older serve reports no ``serve_mode``; it is read as dev, never prod.

    Pure mapping, no serve needed. Telling an operator their app is
    production-served when it is not is the expensive direction of this mistake.
    """
    from kortecx.v1 import gateway_pb2 as _g

    s = HostedAppStatus.from_proto(
        _g.HostedAppStatus(handle="a/b/c", state=5, url="http://127.0.0.1:1", port=1)
    )
    assert s.serve_mode == "dev"
    assert s.state == "running"
