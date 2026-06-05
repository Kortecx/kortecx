"""The async client: invoke a recipe and await the durable result.

kx serve --journal /tmp/kx.db --content /tmp/kx-blobs --dev-allow-local
python examples/async_invoke.py
"""

from __future__ import annotations

import asyncio

from kortecx import AsyncKxClient


async def main() -> None:
    async with AsyncKxClient("http://127.0.0.1:50151") as kx:
        # wait_mode="events" reacts to the live event stream (lower latency than polling).
        result = await kx.invoke(
            "kx/recipes/echo", {"topic": "async hi"}, wait=True, wait_mode="events"
        )
        print("state", result.state, "instance", result.instance_id)


if __name__ == "__main__":
    asyncio.run(main())
