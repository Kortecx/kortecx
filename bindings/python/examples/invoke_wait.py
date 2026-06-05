"""Call the runtime like a function: invoke a recipe and wait for the result.

Start a gateway first:
    kx serve --journal /tmp/kx.db --content /tmp/kx-blobs --dev-allow-local

Then:
    python examples/invoke_wait.py
"""

from __future__ import annotations

from kortecx import KxClient


def main() -> None:
    with KxClient("http://127.0.0.1:50151") as kx:
        result = kx.invoke("kx/recipes/echo", {"topic": "hello, kortecx"}, wait=True)
        print("state           ", result.state)
        print("instance_id     ", result.instance_id)
        print("terminal_mote_id", result.terminal_mote_id)
        print("result_ref      ", result.result_ref)
        # The demo recipe's bytes are binary; .text is None, .bytes has the payload.
        print("result bytes    ", (result.bytes or b"")[:32], "…")


if __name__ == "__main__":
    main()
