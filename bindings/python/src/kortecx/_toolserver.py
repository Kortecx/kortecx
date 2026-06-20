"""A minimal stdio MCP server for ``@kx.tool`` local functions (Batch V2b).

The runtime SPAWNS this (``python -m kortecx._toolserver --script <abs> --tools
a,b``) when it dials an SDK-registered local tool server. It re-loads the user's
module to recover the decorated functions, then speaks newline-delimited JSON-RPC
2.0 over stdin/stdout — the same wire the runtime's ``StdioSession`` drives
(``initialize`` → ``tools/list`` → ``tools/call``). Hand-rolled (the SDK has no
``mcp`` dependency); templated on the in-repo ``kx-mcp`` test stdio servers.

Re-import: the module is loaded under a NON-``__main__`` run name, so a user's
``if __name__ == "__main__":`` block (e.g. their ``.run()`` calls) never re-runs;
only the top-level ``@kx.tool`` decorations register. The ``KX_TOOLSERVE`` env is
set first so the lazy default client refuses to start a run in this context.
"""

from __future__ import annotations

import json
import os
import runpy
import sys
from typing import Any, Dict, List, Optional

from .tools import TOOLSERVE_ENV, LocalToolDef

#: The MCP revision we advertise; the runtime negotiates and never hard-gates.
_PROTOCOL_VERSION = "2026-07-28"


def _load_tools(
    script: Optional[str], module: Optional[str], names: List[str]
) -> Dict[str, LocalToolDef]:
    """Re-import the user's module (registering its ``@kx.tool`` decorations) and
    return the requested tools by name."""
    os.environ[TOOLSERVE_ENV] = "1"
    # Importing here (not at module top) so the env sentinel is set first.
    from . import tools as _tools

    if script:
        # Run the script's top level under a non-__main__ name so a guarded main
        # block is skipped; the decorators populate `_tools._LOCAL_TOOLS`.
        runpy.run_path(script, run_name="__kx_tools__")
    elif module:
        import importlib

        importlib.import_module(module)
    selected: Dict[str, LocalToolDef] = {}
    for name in names:
        tdef = _tools._LOCAL_TOOLS.get(name)
        if tdef is not None:
            selected[name] = tdef
    return selected


def _ok(req_id: Any, result: Any) -> str:
    return json.dumps({"jsonrpc": "2.0", "id": req_id, "result": result}, ensure_ascii=False)


def _err(req_id: Any, code: int, message: str) -> str:
    return json.dumps(
        {"jsonrpc": "2.0", "id": req_id, "error": {"code": code, "message": message}},
        ensure_ascii=False,
    )


def _tools_list_result(tools: Dict[str, LocalToolDef]) -> Dict[str, Any]:
    return {
        "tools": [
            {"name": t.name, "description": t.description, "inputSchema": t.schema}
            for t in tools.values()
        ]
    }


def _handle(req: Dict[str, Any], tools: Dict[str, LocalToolDef]) -> Optional[str]:
    """Answer ONE JSON-RPC request, or ``None`` for a notification (no ``id``)."""
    method = req.get("method")
    if "id" not in req:
        return None  # a notification (e.g. notifications/initialized) — no reply
    req_id = req.get("id")
    if method == "initialize":
        return _ok(
            req_id,
            {
                "protocolVersion": _PROTOCOL_VERSION,
                "capabilities": {},
                "serverInfo": {"name": "kortecx-local-tools", "version": "1"},
            },
        )
    if method == "tools/list":
        return _ok(req_id, _tools_list_result(tools))
    if method == "tools/call":
        params = req.get("params") or {}
        name = params.get("name")
        tdef = tools.get(name) if isinstance(name, str) else None
        if tdef is None:
            return _err(req_id, -32602, f"no such tool: {name}")
        args = params.get("arguments") or {}
        if not isinstance(args, dict):
            return _err(req_id, -32602, "arguments must be a JSON object")
        try:
            result = tdef.fn(**args)
        except Exception as exc:  # the tool body failed — surface it as a tool error
            return _err(req_id, -32000, f"{type(exc).__name__}: {exc}")
        try:
            # The runtime extracts the JSON-RPC `result` field verbatim as the
            # tool's committed output.
            json.dumps(result)
        except (TypeError, ValueError):
            return _err(req_id, -32000, "tool returned a non-JSON-serializable value")
        return _ok(req_id, result)
    return _err(req_id, -32601, f"no such method: {method}")


def _parse_args(argv: List[str]) -> Dict[str, Any]:
    script: Optional[str] = None
    module: Optional[str] = None
    names: List[str] = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--script" and i + 1 < len(argv):
            script = argv[i + 1]
            i += 2
        elif a == "--module" and i + 1 < len(argv):
            module = argv[i + 1]
            i += 2
        elif a == "--tools" and i + 1 < len(argv):
            names = [n for n in argv[i + 1].split(",") if n]
            i += 2
        else:
            i += 1
    return {"script": script, "module": module, "names": names}


def main(argv: Optional[List[str]] = None) -> int:
    opts = _parse_args(sys.argv[1:] if argv is None else argv)
    tools = _load_tools(opts["script"], opts["module"], opts["names"])

    stdin = sys.stdin
    out = sys.stdout
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except (TypeError, ValueError):
            out.write(_err(0, -32700, "parse error") + "\n")
            out.flush()
            continue
        if not isinstance(req, dict):
            out.write(_err(0, -32600, "invalid request") + "\n")
            out.flush()
            continue
        reply = _handle(req, tools)
        if reply is not None:
            out.write(reply + "\n")
            out.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
