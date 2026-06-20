"""Local function tools (``@kx.tool``, Batch V2b) — unit tests (no server).

Covers the decorator + type-hint→inputSchema mapping, the ``tool`` overload
(decorator vs the back-compat node factory), tool-set splitting, the run-terminal
resolution against a mock client, the Agent routing (frozen pre-flight hint +
dynamic→react-auto), and the hand-rolled stdio MCP server round-trip.
"""

from __future__ import annotations

import enum
import json
import os
import subprocess
import sys
from typing import Any, Dict, List, Literal

import pytest

import kortecx as kx
from kortecx.agent import REACT_AUTO_RECIPE_HANDLE, REACT_RECIPE_HANDLE, Agent
from kortecx.chains import Task
from kortecx.tools import (
    ToolError,
    _server_name_for,
    local_tool_def,
    resolve_local_tools,
    split_tools,
    tool,
)
from kortecx.toolscout import RegisteredTool, RegisteredToolsPage

# --- decorator + schema mapping ----------------------------------------------


def test_decorator_maps_basic_types_and_required() -> None:
    @tool
    def f(a: int, b: str, c: bool, d: int = 7) -> int:
        "A tool."
        return a

    td = local_tool_def(f)
    assert td is not None
    assert td.name == "f" and td.version == "1" and td.description == "A tool."
    assert td.schema == {
        "type": "object",
        "properties": {
            "a": {"type": "integer"},
            "b": {"type": "string"},
            "c": {"type": "boolean"},
            "d": {"type": "integer"},
        },
        "required": ["a", "b", "c"],  # d has a default → not required
    }
    assert f(1, "x", True) == 1  # still directly callable


def test_decorator_underscore_to_hyphen_name() -> None:
    @tool
    def my_cool_tool(x: int) -> int:
        return x

    assert local_tool_def(my_cool_tool).name == "my-cool-tool"


class _Color(str, enum.Enum):
    RED = "red"
    BLUE = "blue"


@tool
def _pick(mode: Literal["fast", "slow"], color: _Color) -> str:
    return mode


def test_decorator_enum_and_literal_map_to_string_enum() -> None:
    # NOTE: module-level imports + def — the real @kx.tool usage where
    # ``get_type_hints`` can resolve the names (a nested def under
    # ``from __future__ import annotations`` cannot resolve locally-imported names).
    schema = local_tool_def(_pick).schema["properties"]
    assert schema["mode"] == {"enum": ["fast", "slow"]}
    assert schema["color"] == {"enum": ["red", "blue"]}


def test_decorator_float_warns_and_maps_to_number() -> None:
    with pytest.warns(UserWarning, match="float"):

        @tool
        def f(x: float) -> float:
            return x

    assert local_tool_def(f).schema["properties"]["x"] == {"type": "number"}


def test_parametrized_decorator() -> None:
    @tool(name="adder", version="3", description="custom")
    def add(a: int, b: int) -> int:
        return a + b

    td = local_tool_def(add)
    assert td.name == "adder" and td.version == "3" and td.description == "custom"


def test_tool_overload_node_factory_still_works() -> None:
    node = tool("srv/echo", "1", q="hi")
    assert isinstance(node, Task)
    assert node.step.kind == "tool"
    assert node.step.tool_contract == {"srv/echo": "1"}


def test_tool_overload_rejects_bad_kwargs() -> None:
    with pytest.raises(ToolError):
        tool(bogus="x")  # type: ignore[call-overload]


# --- tool-set splitting -------------------------------------------------------


def test_split_tools_mixes_strings_and_locals() -> None:
    @tool
    def add(a: int, b: int) -> int:
        return a + b

    strs, locals_ = split_tools(["web-search", add])
    assert strs == ["web-search"]
    assert [d.name for d in locals_] == ["add"]


def test_split_tools_plain_function_is_fail_closed() -> None:
    with pytest.raises(ToolError, match="@kx.tool"):
        split_tools([lambda a: a])  # type: ignore[list-item]


def test_split_tools_mapping_passthrough() -> None:
    strs, locals_ = split_tools({"x": "2"})
    assert strs == {"x": "2"} and locals_ == ()


# --- run-terminal resolution (mock client) ------------------------------------


class _FakeClient:
    def __init__(self) -> None:
        self.registered: List[tuple] = []
        self.invoked: List[tuple] = []

    def register_mcp_server(self, *, name, transport, endpoint, args):  # noqa: ANN001
        self.registered.append((name, transport, endpoint, tuple(args)))

        class _R:
            connection_id = "00" * 16
            discovered = 1
            health = "connected"

        return _R()

    def discover_server_tools(self, *, name):  # noqa: ANN001
        # Mimic the gateway: a discovered tool is namespaced <server>/<remote>.
        tools = [
            RegisteredTool(
                tool_id="ab" * 16,
                tool_name=f"{name}/add",
                tool_version="1",
                kind="Mcp",
                description="Add.",
                idempotency_class="i",
                provenance="HumanAuthored",
                registration_status="Approved",
                server_host="",
                net_scope="none",
                is_builtin=False,
            )
        ]
        return RegisteredToolsPage(tools=tools, has_more=False)

    def invoke(self, handle, args, *, wait=True, timeout=120.0):  # noqa: ANN001
        self.invoked.append((handle, args, wait))
        return "INVOKED"


def _add_tool():  # noqa: ANN202
    @tool
    def add(a: int, b: int) -> int:
        return a + b

    return add


def test_resolve_fills_tool_node_contract() -> None:
    add = _add_tool()
    fc = _FakeClient()
    chain = kx.flow().tool(add, a=2, b=2).to_chain()
    resolve_local_tools(fc, chain)
    sname = _server_name_for(local_tool_def(add).script_path)
    step = chain._iter_steps()[0]
    assert step.kind == "tool"
    assert step.tool_contract == {f"{sname}/add": "1"}
    assert fc.registered[0][0] == sname and fc.registered[0][1] == "stdio"
    assert fc.registered[0][2] == sys.executable
    assert "kortecx._toolserver" in fc.registered[0][3]


def test_resolve_is_idempotent() -> None:
    add = _add_tool()
    fc = _FakeClient()
    chain = kx.flow().tool(add, a=1, b=1).to_chain()
    resolve_local_tools(fc, chain)
    resolve_local_tools(fc, chain)  # second pass: no duplicate, no error
    sname = _server_name_for(local_tool_def(add).script_path)
    assert chain._iter_steps()[0].tool_contract == {f"{sname}/add": "1"}


def test_resolve_agentic_model_step() -> None:
    add = _add_tool()
    fc = _FakeClient()
    chain = kx.flow().agent("do math", tools=[add]).to_chain()
    resolve_local_tools(fc, chain)
    sname = _server_name_for(local_tool_def(add).script_path)
    step = chain._iter_steps()[0]
    assert step.kind == "model" and step.tool_contract == {f"{sname}/add": "1"}


def test_resolve_noop_without_local_tools() -> None:
    fc = _FakeClient()
    chain = kx.flow().agent("hi", tools=["web-search"]).to_chain()
    resolve_local_tools(fc, chain)
    assert fc.registered == []
    assert chain._iter_steps()[0].tool_contract == {"web-search": "1"}


# --- Agent routing ------------------------------------------------------------


def test_agent_frozen_with_tools_raises_preflight_hint() -> None:
    add = _add_tool()
    with pytest.raises(ToolError, match="PR-9b-2"):
        Agent("do math", tools=[add]).run("2+2", client=_FakeClient())


def test_agent_dynamic_with_local_tools_routes_to_react_auto() -> None:
    add = _add_tool()
    fc = _FakeClient()
    Agent("do math", tools=[add], dynamic=True).run("2+2", client=fc)
    # registered the local tool's stdio server + invoked react-AUTO (not plain react)
    assert fc.registered, "local tool should be registered for the dynamic lane"
    assert fc.invoked[0][0] == REACT_AUTO_RECIPE_HANDLE


def test_agent_dynamic_without_tools_uses_plain_react() -> None:
    fc = _FakeClient()
    Agent("just chat", dynamic=True).run("hello", client=fc)
    assert fc.registered == []
    assert fc.invoked[0][0] == REACT_RECIPE_HANDLE


# --- the stdio MCP server round-trip -----------------------------------------


def _toolserver_env() -> Dict[str, str]:
    # Make `python -m kortecx._toolserver` importable from the test's interpreter.
    src_dir = os.path.dirname(os.path.dirname(kx.__file__))
    env = dict(os.environ)
    env["PYTHONPATH"] = src_dir + os.pathsep + env.get("PYTHONPATH", "")
    return env


def test_toolserver_roundtrip_and_skips_main_block(tmp_path: Any) -> None:
    script = tmp_path / "mytools.py"
    script.write_text(
        "import kortecx as kx\n"
        "@kx.tool\n"
        "def add(a: int, b: int) -> int:\n"
        '    "Add."\n'
        "    return a + b\n"
        'if __name__ == "__main__":\n'
        '    print("MAIN_RAN")\n'
    )
    proc = subprocess.Popen(
        [sys.executable, "-m", "kortecx._toolserver", "--script", str(script), "--tools", "add"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=_toolserver_env(),
    )
    reqs = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},  # no id ⇒ no reply
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "add", "arguments": {"a": 2, "b": 40}},
        },
        {
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "nope", "arguments": {}},
        },
    ]
    out, err = proc.communicate("\n".join(json.dumps(r) for r in reqs) + "\n", timeout=30)
    assert "MAIN_RAN" not in out, "the __main__ guard must be skipped on re-import"
    resps = {json.loads(line)["id"]: json.loads(line) for line in out.splitlines() if line.strip()}
    assert set(resps) == {1, 2, 3, 4}, f"a notification must not get a reply: {err}"
    assert resps[1]["result"]["protocolVersion"]
    assert resps[2]["result"]["tools"][0]["name"] == "add"
    assert resps[2]["result"]["tools"][0]["inputSchema"]["required"] == ["a", "b"]
    assert resps[3]["result"] == 42  # the runtime extracts `result` verbatim
    assert resps[4]["error"]["code"] == -32602  # unknown tool


def test_toolserver_surfaces_tool_exception() -> None:
    import tempfile

    with tempfile.NamedTemporaryFile("w", suffix=".py", delete=False) as fh:
        fh.write(
            "import kortecx as kx\n"
            "@kx.tool\n"
            "def boom(x: int) -> int:\n"
            "    raise ValueError('nope')\n"
        )
        path = fh.name
    proc = subprocess.Popen(
        [sys.executable, "-m", "kortecx._toolserver", "--script", path, "--tools", "boom"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=_toolserver_env(),
    )
    req = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "boom", "arguments": {"x": 1}},
    }
    out, _ = proc.communicate(json.dumps(req) + "\n", timeout=30)
    resp = json.loads(out.splitlines()[0])
    assert resp["error"]["code"] == -32000
    assert "ValueError" in resp["error"]["message"]
    os.unlink(path)
