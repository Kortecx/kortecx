"""Local function tools — the ``@kx.tool`` decorator (Batch V2b).

```python
import kortecx as kx

@kx.tool
def add(a: int, b: int) -> int:
    "Add two integers."
    return a + b

# fire it deterministically (works today):
print(kx.flow().tool(add, a=2, b=2).run().text)

# or let a model decide (the steered react-auto lane; needs KX_SERVE_AUTOGRANT=1):
print(kx.Agent("Do the math.", tools=[add], dynamic=True).run("what is 2+2?").text)
```

Decorating a Python function turns it into a real, governed, fireable tool with
**zero new runtime substrate**: the SDK exposes the decorated functions as a local
**stdio MCP server** (:mod:`kortecx._toolserver`) and the runtime DIALS it through
the existing PR-6b MCP gateway (``RegisterMcpServer`` / ``DiscoverServerTools``).
A local tool is just another external MCP tool the runtime fires under a
server-built warrant (SN-8 — the client never supplies a warrant).

**Dev-scoped by design.** The runtime spawns the stdio server subprocess, so the
runtime, this interpreter, and the tool module must be co-located (the SDK user is
the operator on their own machine). Registering a stdio MCP server is the same
host-trusted operation as ``kx connections add --command`` (PR-6b/D81/GR19); V2b
adds no new attack surface. Cloud governs the bridge.

**Three firing lanes (D161, GR15-honest):**

- **deterministic** — ``flow().tool(fn, **args)`` fires ONE tool as a standalone
  node (works today, even model-free).
- **steered / dynamic** — ``Agent(tools=[fn], dynamic=True)`` runs the
  ``kx/recipes/react-auto`` loop where the model picks tools (needs a served model
  + ``KX_SERVE_AUTOGRANT=1``).
- **frozen / deterministic-agentic** — ``Agent(tools=[fn])`` (the default lane) is a
  MODEL step with a fixed tool set; its bounded loop **lands in PR-9b-2** and is
  refused at submit today, so the Agent raises a clear pre-flight hint until then.

**Re-import contract.** The runtime spawns ``python -m kortecx._toolserver`` which
re-loads your module to recover the functions. Decorate at MODULE level and guard
any ``.run()`` calls under ``if __name__ == "__main__":`` (the toolserver loads the
module under a non-``__main__`` name, so a guarded main block never re-runs).
"""

from __future__ import annotations

import enum
import inspect
import os
import sys
import typing
import warnings
from dataclasses import dataclass
from typing import (
    TYPE_CHECKING,
    Any,
    Callable,
    Dict,
    List,
    Mapping,
    Optional,
    Sequence,
    Tuple,
    Union,
)

from .blueprints import TOOL_ARGS_KEY, StepInput

if TYPE_CHECKING:
    from .chains import Chain, Task

#: Env sentinel set by :mod:`kortecx._toolserver` while it re-imports the user's
#: module. The lazy default client refuses to run a flow/agent in this context
#: (a guard against an un-guarded top-level ``.run()`` re-executing + recursing).
TOOLSERVE_ENV = "KX_TOOLSERVE"

#: The stable server-name prefix for an SDK-registered local stdio MCP server.
_LOCAL_SERVER_PREFIX = "kxlocal-"


@dataclass(eq=False)
class LocalToolDef:
    """A function exposed as a local MCP tool (identity = the object). ``fn`` runs
    IN the toolserver subprocess; ``schema`` is the derived MCP ``inputSchema``."""

    name: str
    version: str
    description: str
    schema: Dict[str, Any]
    fn: Callable[..., Any]
    module: str
    qualname: str
    script_path: str


#: Process-global registry the toolserver subprocess reads after re-importing a
#: module (name → def). In the authoring process it simply accumulates decorations.
_LOCAL_TOOLS: Dict[str, LocalToolDef] = {}


# --- type-hint → MCP inputSchema (the gateway's mapper gates str/int/bool/enum) ---


def _type_to_jsonschema(name: str, ann: Any) -> Dict[str, Any]:
    """Map ONE parameter annotation to a JSON-Schema property. Mirrors what the
    gateway's ``json_schema_to_input_schema`` actually type-gates: ``str`` /
    ``int`` / ``bool`` / string-enum. ``float`` maps to ``number`` (the runtime
    does NOT gate floats — a warn mirrors the ``tool()`` no-floats rule); any
    other / missing annotation falls back to ``string`` (the arg passes verbatim)."""
    origin = typing.get_origin(ann)
    if origin is typing.Literal:
        vals = list(typing.get_args(ann))
        if vals and all(isinstance(v, str) for v in vals):
            return {"enum": vals}
        return {"type": "string"}
    if isinstance(ann, type) and issubclass(ann, enum.Enum):
        members = [m.value for m in ann]
        if members and all(isinstance(v, str) for v in members):
            return {"enum": members}
        return {"type": "string"}
    # NOTE: bool is a subclass of int — check it first.
    if ann is bool:
        return {"type": "boolean"}
    if ann is int:
        return {"type": "integer"}
    if ann is str:
        return {"type": "string"}
    if ann is float:
        warnings.warn(
            f"@kx.tool: parameter {name!r} is typed float; the runtime does not "
            "type-gate numbers — it is passed verbatim. Prefer int where possible.",
            stacklevel=4,
        )
        return {"type": "number"}
    # Unknown/complex (list/dict/typed object) — no client gate; document that the
    # function receives the decoded JSON value verbatim.
    return {"type": "string"}


def _schema_from_signature(fn: Callable[..., Any]) -> Dict[str, Any]:
    """Derive an MCP ``inputSchema`` (``{type:object, properties, required}``) from
    ``fn``'s signature + type hints. Params without a default are ``required``;
    ``*args`` / ``**kwargs`` are skipped (un-mappable — documented)."""
    sig = inspect.signature(fn)
    try:
        hints = typing.get_type_hints(fn)
    except Exception:  # pragma: no cover - exotic annotations
        hints = {}
    props: Dict[str, Any] = {}
    required: List[str] = []
    for pname, param in sig.parameters.items():
        if param.kind in (param.VAR_POSITIONAL, param.VAR_KEYWORD):
            continue
        ann = hints.get(pname, param.annotation)
        props[pname] = _type_to_jsonschema(pname, ann)
        if param.default is inspect.Parameter.empty:
            required.append(pname)
    schema: Dict[str, Any] = {"type": "object", "properties": props}
    if required:
        schema["required"] = required
    return schema


def _script_path_of(fn: Callable[..., Any]) -> str:
    """The abs path of the file defining ``fn`` (the toolserver re-loads it). Raise
    a clear error for a function with no source file (a REPL/notebook closure)."""
    src = inspect.getsourcefile(fn) or inspect.getfile(fn)
    if not src:
        raise ToolError(
            f"@kx.tool: cannot locate the source file for {getattr(fn, '__name__', fn)!r}; "
            "a local tool must be a function defined in a .py file (not a REPL closure)."
        )
    return os.path.abspath(src)


class ToolError(ValueError):
    """A local-tool authoring/registration error."""


def _make_local_tool(
    fn: Callable[..., Any],
    *,
    name: Optional[str] = None,
    version: str = "1",
    description: Optional[str] = None,
) -> Callable[..., Any]:
    """Tag ``fn`` with a :class:`LocalToolDef` and register it. Returns ``fn``
    unchanged (still directly callable)."""
    if not callable(fn):
        raise ToolError("@kx.tool must decorate a callable")
    tool_name = name or fn.__name__.replace("_", "-")
    desc = description if description is not None else (inspect.getdoc(fn) or "")
    tdef = LocalToolDef(
        name=tool_name,
        version=str(version),
        description=desc.strip(),
        schema=_schema_from_signature(fn),
        fn=fn,
        module=getattr(fn, "__module__", ""),
        qualname=getattr(fn, "__qualname__", tool_name),
        script_path=_script_path_of(fn),
    )
    _LOCAL_TOOLS[tool_name] = tdef
    fn.__kx_tool__ = tdef  # type: ignore[attr-defined]
    return fn


def local_tool_def(fn: Any) -> Optional[LocalToolDef]:
    """Return the :class:`LocalToolDef` a ``@kx.tool``-decorated ``fn`` carries, or
    ``None`` (not a local tool)."""
    return getattr(fn, "__kx_tool__", None)


# --- the public `tool` symbol: a decorator AND the back-compat node factory -------


@typing.overload
def tool(fn: Callable[..., Any]) -> Callable[..., Any]: ...


@typing.overload
def tool(
    *, name: Optional[str] = ..., version: str = ..., description: Optional[str] = ...
) -> Callable[[Callable[..., Any]], Callable[..., Any]]: ...


@typing.overload
def tool(tool_id: str, tool_version: str, **args: object) -> "Task": ...


def tool(*args: Any, **kwargs: Any) -> Any:
    """``@kx.tool`` — expose a local function as a governed tool (V2b); OR
    ``tool("id", "version", **args)`` — a standalone TOOL node firing a REGISTERED
    tool (PR-6b-2, the back-compat factory).

    Decorator forms::

        @kx.tool
        def add(a: int, b: int) -> int: ...

        @kx.tool(name="adder", version="2")
        def add(a: int, b: int) -> int: ...

    The function's type hints become the tool's MCP ``inputSchema`` (str / int /
    bool / string-enum are type-gated by the runtime; floats pass verbatim).
    """
    # Bare decorator: @kx.tool
    if len(args) == 1 and callable(args[0]) and not kwargs:
        return _make_local_tool(args[0])
    # Node factory (back-compat): tool("id", "version", **args)
    if args and isinstance(args[0], str):
        from .chains import tool as _node_tool

        return _node_tool(*args, **kwargs)
    # Parametrized decorator: @kx.tool(name=..., version=..., description=...)
    if not args:
        name = kwargs.pop("name", None)
        version = kwargs.pop("version", "1")
        description = kwargs.pop("description", None)
        if kwargs:
            raise ToolError(f"@kx.tool got unexpected keyword(s): {sorted(kwargs)}")

        def _decorate(fn: Callable[..., Any]) -> Callable[..., Any]:
            return _make_local_tool(fn, name=name, version=version, description=description)

        return _decorate
    raise ToolError(
        "tool(...) takes either @kx.tool on a function, @kx.tool(name=..., version=...), "
        'or the node factory tool("id", "version", **args)'
    )


# --- run-terminal resolution: register the stdio server(s) + name the tools -------

ToolsArg = Optional[Union[Sequence[Any], Mapping[str, str]]]


def split_tools(tools: ToolsArg) -> Tuple[ToolsArg, Tuple[LocalToolDef, ...]]:
    """Split a ``tools=`` value into (string/mapping grants, local-tool defs). A bare
    function must be ``@kx.tool``-decorated; a plain lambda is a fail-closed error."""
    if tools is None or isinstance(tools, Mapping):
        return tools, ()
    names: List[Any] = []
    locals_: List[LocalToolDef] = []
    for t in tools:
        tdef = local_tool_def(t)
        if tdef is not None:
            locals_.append(tdef)
        elif callable(t):
            raise ToolError(
                "a plain function in tools=[...] must be decorated with @kx.tool "
                f"(got {getattr(t, '__name__', t)!r})"
            )
        else:
            names.append(t)
    return (names if names else None), tuple(locals_)


def local_tool_node(tdef: LocalToolDef, args: Mapping[str, object]) -> "Task":
    """A standalone TOOL node for a local function — the tool_contract is filled at
    resolution (the namespaced ``<server>/<name>`` is server-derived)."""
    from .chains import Task, _canonical_args_json

    return Task(
        StepInput(
            kind="tool",
            tool_contract={},
            params={TOOL_ARGS_KEY: _canonical_args_json(dict(args))},
            local_tools=(tdef,),
        )
    )


def _server_name_for(script_path: str) -> str:
    """A stable, deterministic server name per defining script (so re-runs upsert
    the same connection — ``connection_id_of(name)`` is deterministic, SN-8)."""
    import hashlib

    digest = hashlib.blake2b(script_path.encode("utf-8"), digest_size=8).hexdigest()
    return f"{_LOCAL_SERVER_PREFIX}{digest}"


def _server_args(script_path: str, names: Sequence[str]) -> List[str]:
    return [
        "-m",
        "kortecx._toolserver",
        "--script",
        script_path,
        "--tools",
        ",".join(sorted(set(names))),
    ]


def _plan_registrations(
    defs: Sequence[LocalToolDef],
) -> List[Tuple[str, str, List[str]]]:
    """Group defs by defining script → one stdio server each. Returns
    ``(server_name, script_path, [tool_names])`` per server, deterministically."""
    by_script: Dict[str, List[LocalToolDef]] = {}
    for d in defs:
        by_script.setdefault(d.script_path, []).append(d)
    plan: List[Tuple[str, str, List[str]]] = []
    for script in sorted(by_script):
        names = [d.name for d in by_script[script]]
        plan.append((_server_name_for(script), script, names))
    return plan


def _resolved_name(server_name: str, tool_name: str, registered: Sequence[Any]) -> str:
    """Find the namespaced ``<server>/<name>`` for ``tool_name`` among a server's
    discovered tools (read back — never guess the namespacing rule)."""
    want = f"{server_name}/{tool_name}"
    for rt in registered:
        if getattr(rt, "tool_name", None) == want:
            return rt.tool_name
    # Fall back to a remote-segment match (defensive against any normalization).
    for rt in registered:
        full = getattr(rt, "tool_name", "")
        if "/" in full and full.split("/", 1)[1] == tool_name:
            return full
    raise ToolError(
        f"local tool {tool_name!r} was not discovered on server {server_name!r} "
        "(registration/discovery mismatch)"
    )


def _apply_contract(chain: "Chain", name_map: Dict[int, str]) -> None:
    """Augment each step's ``tool_contract`` with its resolved local-tool names
    (idempotent ``setdefault`` — safe to re-run)."""
    for step in chain._iter_steps():
        for tdef in step.local_tools:
            resolved = name_map.get(id(tdef))
            if resolved is not None:
                step.tool_contract.setdefault(resolved, tdef.version)


def resolve_local_tools(client: Any, chain: "Chain") -> None:
    """Register every local tool referenced by ``chain`` (sync) + rewrite each
    step's ``tool_contract`` to the server-derived names. No-op when there are none."""
    defs = _collect_defs(chain)
    if not defs:
        return
    by_def: Dict[str, List[LocalToolDef]] = {}
    for d in defs:
        by_def.setdefault(d.script_path, []).append(d)
    name_map: Dict[int, str] = {}
    for server_name, script, names in _plan_registrations(defs):
        client.register_mcp_server(
            name=server_name,
            transport="stdio",
            endpoint=sys.executable,
            args=_server_args(script, names),
        )
        page = client.discover_server_tools(name=server_name)
        for tdef in by_def[script]:
            name_map[id(tdef)] = _resolved_name(server_name, tdef.name, page.tools)
    _apply_contract(chain, name_map)


async def aresolve_local_tools(client: Any, chain: "Chain") -> None:
    """As :func:`resolve_local_tools`, over an async client."""
    defs = _collect_defs(chain)
    if not defs:
        return
    by_def: Dict[str, List[LocalToolDef]] = {}
    for d in defs:
        by_def.setdefault(d.script_path, []).append(d)
    name_map: Dict[int, str] = {}
    for server_name, script, names in _plan_registrations(defs):
        await client.register_mcp_server(
            name=server_name,
            transport="stdio",
            endpoint=sys.executable,
            args=_server_args(script, names),
        )
        page = await client.discover_server_tools(name=server_name)
        for tdef in by_def[script]:
            name_map[id(tdef)] = _resolved_name(server_name, tdef.name, page.tools)
    _apply_contract(chain, name_map)


def _collect_defs(chain: "Chain") -> List[LocalToolDef]:
    seen: Dict[int, LocalToolDef] = {}
    for step in chain._iter_steps():
        for tdef in step.local_tools:
            seen.setdefault(id(tdef), tdef)
    return list(seen.values())


def register_tools(client: Any, tools: ToolsArg) -> None:
    """Register the local tools in a ``tools=`` set (sync) WITHOUT rewriting a
    contract — used by the dynamic react-auto lane, which auto-grants the live
    registry. No-op when there are no local tools."""
    _, defs = split_tools(tools)
    if not defs:
        return
    for server_name, script, names in _plan_registrations(defs):
        client.register_mcp_server(
            name=server_name,
            transport="stdio",
            endpoint=sys.executable,
            args=_server_args(script, names),
        )


async def aregister_tools(client: Any, tools: ToolsArg) -> None:
    """As :func:`register_tools`, over an async client."""
    _, defs = split_tools(tools)
    if not defs:
        return
    for server_name, script, names in _plan_registrations(defs):
        await client.register_mcp_server(
            name=server_name,
            transport="stdio",
            endpoint=sys.executable,
            args=_server_args(script, names),
        )
