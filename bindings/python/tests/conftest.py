"""Test fixtures that spin up a real ``kx serve`` gateway.

The contract tests drive the SDK against an actual embedded single-system runtime
(the FFI-free `kx` binary) and compare results to the `kx` CLI, byte-for-byte.
This is the conformance guarantee: the Python SDK and the reference CLI agree on
every server-derived id and result.

The `kx` binary is located via (in order): the ``KX_BIN`` env var, the workspace
``target/release/kx``, then ``target/debug/kx``; if none exist it is built FFI-free
(``cargo build --release -p kx-cli``). The demo recipe ``kx/recipes/echo`` is
provisioned by the gateway at startup, so no catalog setup is needed.
"""

from __future__ import annotations

import os
import pathlib
import socket
import subprocess
import time
from dataclasses import dataclass
from typing import List, Optional

import grpc
import pytest

REPO_ROOT = pathlib.Path(__file__).resolve().parents[3]
ECHO_HANDLE = "kx/recipes/echo"


def _find_or_build_kx() -> str:
    env = os.environ.get("KX_BIN")
    if env and pathlib.Path(env).exists():
        return env
    # NOTE: a pre-existing binary is used as-is. The dataset contract tests need a
    # binary built ``--features hnsw``; a stale non-hnsw ``target/release/kx`` makes
    # them fail with UNIMPLEMENTED — ``rm`` it (or set KX_BIN). CI builds fresh with it.
    for rel in ("target/release/kx", "target/debug/kx"):
        cand = REPO_ROOT / rel
        if cand.exists():
            return str(cand)
    # Build it FFI-free (no C++ toolchain needed). `--features hnsw` adds the Datasets
    # data-plane (RAG) — still pure-Rust (kx-dataset-hnsw + rusqlite, no llama.cpp) — so
    # the contract tests can exercise the client-vector ingest/query path.
    subprocess.run(
        ["cargo", "build", "--release", "-p", "kx-cli", "--features", "hnsw"],
        cwd=REPO_ROOT,
        check=True,
    )
    return str(REPO_ROOT / "target/release/kx")


def _free_port() -> int:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


@dataclass
class Server:
    """A running ``kx serve`` under test."""

    endpoint: str
    ws_endpoint: str
    proc: subprocess.Popen
    token: Optional[str] = None

    def stop(self) -> None:
        self.proc.terminate()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:  # pragma: no cover
            self.proc.kill()


def _wait_ready(endpoint: str, proc: subprocess.Popen, timeout: float = 40.0) -> None:
    target = endpoint[len("http://") :]
    deadline = time.time() + timeout
    while time.time() < deadline:
        if proc.poll() is not None:
            out = proc.stdout.read().decode(errors="replace") if proc.stdout else ""
            raise RuntimeError(f"kx serve exited early (code {proc.returncode}):\n{out}")
        channel = grpc.insecure_channel(target)
        try:
            grpc.channel_ready_future(channel).result(timeout=0.5)
            channel.close()
            return
        except grpc.FutureTimeoutError:
            channel.close()
        time.sleep(0.1)
    raise RuntimeError("kx serve did not become ready in time")


def _spawn(kx_bin: str, tmp: pathlib.Path, extra: List[str]) -> Server:
    port, ws_port = _free_port(), _free_port()
    endpoint = f"http://127.0.0.1:{port}"
    ws_endpoint = f"ws://127.0.0.1:{ws_port}"
    args = [
        kx_bin,
        "serve",
        "--journal",
        str(tmp / "kx.db"),
        "--content",
        str(tmp / "blobs"),
        "--listen",
        f"127.0.0.1:{port}",
        "--ws-listen",
        f"127.0.0.1:{ws_port}",
        *extra,
    ]
    proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    _wait_ready(endpoint, proc)
    return Server(endpoint=endpoint, ws_endpoint=ws_endpoint, proc=proc)


@pytest.fixture(scope="session")
def kx_bin() -> str:
    return _find_or_build_kx()


@pytest.fixture()
def serve(kx_bin: str, tmp_path_factory):
    """A factory that spawns (and later tears down) gateways with given flags."""
    servers: List[Server] = []

    def _make(*extra: str) -> Server:
        tmp = tmp_path_factory.mktemp("kxsrv")
        s = _spawn(kx_bin, tmp, list(extra))
        servers.append(s)
        return s

    try:
        yield _make
    finally:
        for s in servers:
            s.stop()


@pytest.fixture()
def dev_server(kx_bin: str, tmp_path) -> Server:
    """A loopback ``--dev-allow-local`` gateway (no token needed)."""
    server = _spawn(kx_bin, tmp_path, ["--dev-allow-local"])
    try:
        yield server
    finally:
        server.stop()


@pytest.fixture()
def auth_server(kx_bin: str, tmp_path) -> Server:
    """A token-authenticated gateway (``--auth-token s3cr3t=alice``)."""
    server = _spawn(kx_bin, tmp_path, ["--auth-token", "s3cr3t=alice"])
    server.token = "s3cr3t"
    try:
        yield server
    finally:
        server.stop()
