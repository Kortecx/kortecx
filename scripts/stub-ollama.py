#!/usr/bin/env python3
"""stub-ollama.py — a loopback stand-in for the Ollama daemon's DISCOVERY surface.

Serves exactly what `kx serve`'s Ollama auto-detection reads at boot
(kx-ollama's OllamaClient):

    GET  /api/version -> {"version": ...}
    GET  /api/tags    -> {"models": [{"name": <tag>}]}
    GET  /api/ps      -> {"models": []}
    POST /api/show    -> details.family + model_info.<arch>.context_length

and NOTHING else — /api/generate is a 404, so generation can never be asserted
against this stub, only registration/seeding/bind. That is the point: the
`verify-release-parity` gate needs `serve_model.is_some()` (every bundled
capability and agent recipe is gated on a resolved serve model), and this makes
that leg hermetic and deterministic on CI. The real-dialect proof lives in
`verify-release-live`, which runs against a real Ollama daemon and a real model.

Usage:  stub-ollama.py <port> [tag]
        (binds 127.0.0.1:<port>; default tag kx-stub-model:latest)
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TAG = sys.argv[2] if len(sys.argv) > 2 else "kx-stub-model:latest"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _reply(self, obj):
        body = json.dumps(obj).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _missing(self):
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        if self.path == "/api/version":
            self._reply({"version": "0.0.0-kx-stub"})
        elif self.path == "/api/tags":
            self._reply({"models": [{"name": TAG}]})
        elif self.path == "/api/ps":
            self._reply({"models": []})
        else:
            self._missing()

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        self.rfile.read(length)
        if self.path == "/api/show":
            self._reply(
                {
                    "details": {"family": "gemma"},
                    "model_info": {"gemma3.context_length": 8192},
                    "capabilities": [],
                }
            )
        else:
            self._missing()

    def log_message(self, fmt, *args):  # keep the gate's output clean
        pass


def main():
    port = int(sys.argv[1])
    # Threading: HTTP/1.1 keep-alive means a pooled client holds its connection
    # open idle, and a single-connection server would then block any second
    # connection until the gate's serve timeout turned it into a misleading red.
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"stub-ollama: serving tag {TAG} on 127.0.0.1:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
