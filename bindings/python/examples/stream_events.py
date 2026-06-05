"""Start a run, then watch its durable event deltas as they commit.

kx serve --journal /tmp/kx.db --content /tmp/kx-blobs --dev-allow-local
python examples/stream_events.py
"""

from __future__ import annotations

from kortecx import KxClient


def main() -> None:
    with KxClient("http://127.0.0.1:50151") as kx:
        run = kx.invoke("kx/recipes/echo", {"topic": "watch me"})  # no wait → a handle
        print("watching run", run.instance_id)
        # One snapshot (since the start, to the current journal boundary):
        for delta in run.events(since=0, follow=False):
            print(f"  seq {delta.seq:>4}  {delta.kind:<13} {delta.mote_id or delta.target_mote_id}")
        # For a live tail until Ctrl-C, pass follow=True.


if __name__ == "__main__":
    main()
