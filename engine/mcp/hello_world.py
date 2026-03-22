# Simple hello world MCP server for testing connectivity
"""A minimal MCP server that exposes a greeting tool."""

import json
import sys


def handle_request(request: dict) -> dict:
    """Handle an MCP tool call request."""
    method = request.get("method", "")

    if method == "tools/list":
        return {
            "tools": [
                {
                    "name": "greet",
                    "description": "Returns a friendly greeting",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name to greet",
                            }
                        },
                        "required": ["name"],
                    },
                }
            ]
        }

    if method == "tools/call":
        tool_name = request.get("params", {}).get("name", "")
        args = request.get("params", {}).get("arguments", {})

        if tool_name == "greet":
            name = args.get("name", "World")
            return {"content": [{"type": "text", "text": f"Hello, {name}! Welcome to Kortecx MCP."}]}

        return {"error": {"code": -32601, "message": f"Unknown tool: {tool_name}"}}

    return {"error": {"code": -32601, "message": f"Unknown method: {method}"}}


if __name__ == "__main__":
    # Self-test: verify the server responds correctly
    print("MCP Hello World Server — self-test")

    # Test tool listing
    result = handle_request({"method": "tools/list"})
    assert len(result["tools"]) == 1
    assert result["tools"][0]["name"] == "greet"
    print("[PASS] tools/list")

    # Test tool call
    result = handle_request(
        {
            "method": "tools/call",
            "params": {"name": "greet", "arguments": {"name": "Kortecx"}},
        }
    )
    assert "Hello, Kortecx!" in result["content"][0]["text"]
    print("[PASS] tools/call greet")

    print("All tests passed.")
