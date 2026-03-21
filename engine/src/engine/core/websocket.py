from __future__ import annotations

import json
import logging
from datetime import UTC, datetime
from typing import Any

from fastapi import APIRouter, WebSocket, WebSocketDisconnect

logger = logging.getLogger("engine.ws")


class WebSocketManager:
    """Manages WebSocket connections with pub/sub channels."""

    def __init__(self) -> None:
        self.active: dict[str, WebSocket] = {}
        self.subscriptions: dict[str, set[str]] = {}  # channel -> set of conn ids
        self.router = APIRouter()
        self._register_routes()

    def _register_routes(self) -> None:
        @self.router.websocket("/ws")
        async def websocket_endpoint(websocket: WebSocket):
            await self.connect(websocket)

    async def connect(self, ws: WebSocket) -> None:
        await ws.accept()
        conn_id = str(id(ws))
        self.active[conn_id] = ws
        logger.info("WebSocket connected: %s", conn_id)
        try:
            while True:
                raw = await ws.receive_text()
                msg = json.loads(raw)
                await self._handle_message(conn_id, msg)
        except WebSocketDisconnect:
            self._remove(conn_id)
            logger.info("WebSocket disconnected: %s", conn_id)

    async def _handle_message(self, conn_id: str, msg: dict[str, Any]) -> None:
        event = msg.get("event", "")
        if event == "subscribe":
            channel = msg.get("channel", "")
            self.subscriptions.setdefault(channel, set()).add(conn_id)
        elif event == "unsubscribe":
            channel = msg.get("channel", "")
            self.subscriptions.get(channel, set()).discard(conn_id)
        elif event == "ping":
            ws = self.active.get(conn_id)
            if ws:
                await ws.send_json({"event": "pong", "timestamp": datetime.now(UTC).isoformat()})
        elif event.startswith("quorum."):
            await self._handle_quorum_event(conn_id, event, msg)
        elif event == "workflow.execute":
            await self._handle_workflow_execute(conn_id, msg)

    async def _handle_quorum_event(self, conn_id: str, event: str, msg: dict[str, Any]) -> None:
        """Route quorum.* events to the QuorumHandler."""
        from engine.routers.quorum import quorum_handler

        if quorum_handler is None:
            await self.send_to(conn_id, "quorum.error", {"event": event, "error": "Quorum service not initialized"})
            return

        data = msg.get("data", {})
        try:
            response = await quorum_handler.handle(conn_id, event, data)
            if response is not None:
                await self.send_to(conn_id, f"{event}.result", response)
        except Exception as e:
            logger.error("Quorum event %s failed: %s", event, e)
            await self.send_to(conn_id, "quorum.error", {"event": event, "error": str(e)})

    async def _handle_workflow_execute(self, conn_id: str, msg: dict[str, Any]) -> None:
        """Handle workflow execution triggered via WebSocket."""
        import asyncio

        from engine.services.orchestrator import (
            StepConfig,
            StepIntegration,
            WorkflowRequest,
            orchestrator,
        )

        data = msg.get("data", {})
        name = data.get("name", "")
        if not name:
            await self.send_to(conn_id, "workflow.failed", {"error": "Workflow name is required"})
            return

        steps_raw = data.get("steps", [])
        if not steps_raw:
            await self.send_to(conn_id, "workflow.failed", {"error": "At least one step is required"})
            return

        request = WorkflowRequest(
            workflow_id=data.get("workflowId", ""),
            name=name,
            goal_file_url=data.get("goalFileUrl", ""),
            input_file_urls=data.get("inputFileUrls", []),
            steps=[
                StepConfig(
                    step_id=s.get("stepId", ""),
                    expert_id=s.get("expertId"),
                    task_description=s.get("taskDescription", ""),
                    step_name=s.get("name", "") or s.get("stepId", ""),
                    model_source=s.get("modelSource", "local"),
                    local_model=s.get("localModel"),
                    temperature=s.get("temperature", 0.7),
                    max_tokens=s.get("maxTokens", 4096),
                    connection_type=s.get("connectionType", "sequential"),
                    system_instructions=s.get("systemInstructions", ""),
                    voice_command=s.get("voiceCommand", ""),
                    file_locations=s.get("fileLocations", []),
                    step_file_names=s.get("stepFileNames", []),
                    step_image_names=s.get("stepImageNames", []),
                    integrations=[
                        StepIntegration(
                            id=si.get("id", ""),
                            type=si.get("type", "integration"),
                            reference_id=si.get("referenceId", ""),
                            name=si.get("name", ""),
                            icon=si.get("icon", ""),
                            color=si.get("color", ""),
                            config=si.get("config", {}),
                        )
                        for si in s.get("integrations", [])
                    ],
                )
                for s in steps_raw
            ],
        )

        # Auto-subscribe the sender to the workflow channel
        run_channel = f"workflow.{request.workflow_id}"
        self.subscriptions.setdefault(run_channel, set()).add(conn_id)

        # Run in background
        asyncio.create_task(orchestrator.execute_workflow(request))

    async def broadcast(self, channel: str, event: str, data: Any) -> None:
        """Broadcast an event to all subscribers of a channel."""
        payload = json.dumps(
            {
                "event": event,
                "channel": channel,
                "data": data,
                "timestamp": datetime.now(UTC).isoformat(),
            }
        )
        dead: list[str] = []
        for conn_id in self.subscriptions.get(channel, set()):
            ws = self.active.get(conn_id)
            if ws:
                try:
                    await ws.send_text(payload)
                except Exception:
                    dead.append(conn_id)
        for cid in dead:
            self._remove(cid)

    async def send_to(self, conn_id: str, event: str, data: Any) -> None:
        ws = self.active.get(conn_id)
        if ws:
            await ws.send_json(
                {
                    "event": event,
                    "data": data,
                    "timestamp": datetime.now(UTC).isoformat(),
                }
            )

    def _remove(self, conn_id: str) -> None:
        self.active.pop(conn_id, None)
        for subs in self.subscriptions.values():
            subs.discard(conn_id)

    async def disconnect_all(self) -> None:
        for ws in list(self.active.values()):
            try:
                await ws.close()
            except Exception:
                pass
        self.active.clear()
        self.subscriptions.clear()


ws_manager = WebSocketManager()
