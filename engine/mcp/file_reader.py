# File reader MCP server — read and search local files
"""MCP server that exposes file reading and content search tools."""

import json
import os
import sys
from pathlib import Path


ALLOWED_DIR = os.environ.get("MCP_FILE_ROOT", ".")


def handle_request(request: dict) -> dict:
    """Handle an MCP tool call request."""
    method = request.get("method", "")

    if method == "tools/list":
        return {
            "tools": [
                {
                    "name": "read_file",
                    "description": "Read the contents of a file",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Relative file path to read",
                            }
                        },
                        "required": ["path"],
                    },
                },
                {
                    "name": "list_files",
                    "description": "List files in a directory",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "directory": {
                                "type": "string",
                                "description": "Relative directory path",
                                "default": ".",
                            },
                            "pattern": {
                                "type": "string",
                                "description": "Glob pattern to filter files",
                                "default": "*",
                            },
                        },
                    },
                },
            ]
        }

    if method == "tools/call":
        tool_name = request.get("params", {}).get("name", "")
        args = request.get("params", {}).get("arguments", {})

        if tool_name == "read_file":
            rel_path = args.get("path", "")
            full = Path(ALLOWED_DIR) / rel_path
            if not full.resolve().is_relative_to(Path(ALLOWED_DIR).resolve()):
                return {"error": {"code": -1, "message": "Path traversal blocked"}}
            if not full.is_file():
                return {"content": [{"type": "text", "text": f"File not found: {rel_path}"}]}
            try:
                text = full.read_text(encoding="utf-8", errors="replace")
                return {"content": [{"type": "text", "text": text}]}
            except Exception as e:
                return {"content": [{"type": "text", "text": f"Error reading file: {e}"}]}

        if tool_name == "list_files":
            directory = args.get("directory", ".")
            pattern = args.get("pattern", "*")
            full = Path(ALLOWED_DIR) / directory
            if not full.resolve().is_relative_to(Path(ALLOWED_DIR).resolve()):
                return {"error": {"code": -1, "message": "Path traversal blocked"}}
            if not full.is_dir():
                return {"content": [{"type": "text", "text": f"Directory not found: {directory}"}]}
            files = sorted(str(f.relative_to(full)) for f in full.glob(pattern) if f.is_file())
            return {"content": [{"type": "text", "text": "\n".join(files) if files else "(empty)"}]}

        return {"error": {"code": -32601, "message": f"Unknown tool: {tool_name}"}}

    return {"error": {"code": -32601, "message": f"Unknown method: {method}"}}


if __name__ == "__main__":
    print("MCP File Reader Server — self-test")

    result = handle_request({"method": "tools/list"})
    assert len(result["tools"]) == 2
    print("[PASS] tools/list")

    result = handle_request(
        {
            "method": "tools/call",
            "params": {"name": "list_files", "arguments": {"directory": ".", "pattern": "*.py"}},
        }
    )
    assert "content" in result
    print("[PASS] list_files")

    print("All tests passed.")
