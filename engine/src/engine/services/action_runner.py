"""Action runner — executes action steps with optional Docker-containerised transformation."""

from __future__ import annotations

import asyncio
import logging
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from engine.services.step_artifacts import step_artifacts

logger = logging.getLogger("engine.action_runner")

STEPS_EXECUTION_ROOT = Path(__file__).resolve().parents[3] / "steps" / "execution"

# Container names matching docker-compose.yml
PYTHON_CONTAINER = "kortecx_executor_python"
TS_CONTAINER = "kortecx_executor_ts"

# Extension → (container, interpreter command)
_LANG_MAP: dict[str, tuple[str, list[str]]] = {
    ".py": (PYTHON_CONTAINER, ["python"]),
    ".ts": (TS_CONTAINER, ["npx", "tsx"]),
    ".js": (TS_CONTAINER, ["node"]),
    ".mjs": (TS_CONTAINER, ["node"]),
}


@dataclass
class ActionResult:
    """Result of an action step execution."""

    file_path: str
    file_name: str
    mime_type: str
    size_bytes: int
    output_format: str
    success: bool = True
    error: str = ""


class ActionRunner:
    """Execute action steps — file generation with optional container-based transformation.

    Transformer types:
      - none:       Write previous output directly as markdown or convert to PDF on the host.
      - mcp:        Run an MCP server script inside a Docker executor container.
      - executable: Run an arbitrary script inside a Docker executor container.
    """

    # ── Public API ────────────────────────────────────────────────────────────

    async def run(
        self,
        previous_output: str,
        action_config: dict[str, Any],
        workflow_name: str,
        step_name: str,
        run_id: str,
    ) -> ActionResult:
        """Run an action step and return the generated file information."""
        output_format: str = action_config.get("outputFormat", "markdown")
        output_filename: str = action_config.get("outputFilename", "output.md")
        transformer_type: str = action_config.get("transformerType", "none")

        # Ensure the output directory exists
        step_dir = step_artifacts.get_step_dir(workflow_name, step_name)

        try:
            if transformer_type == "none":
                return await self._run_direct(previous_output, output_format, output_filename, step_dir)
            elif transformer_type in ("mcp", "executable"):
                return await self._run_in_container(previous_output, action_config, output_format, output_filename, step_dir)
            else:
                return ActionResult(
                    file_path="",
                    file_name=output_filename,
                    mime_type="",
                    size_bytes=0,
                    output_format=output_format,
                    success=False,
                    error=f"Unknown transformer type: {transformer_type}",
                )
        except Exception as exc:
            logger.exception("Action step failed: %s", exc)
            return ActionResult(
                file_path="",
                file_name=output_filename,
                mime_type="",
                size_bytes=0,
                output_format=output_format,
                success=False,
                error=str(exc),
            )

    # ── Direct (none) transformer — runs on host ──────────────────────────────

    async def _run_direct(
        self,
        content: str,
        output_format: str,
        output_filename: str,
        step_dir: Path,
    ) -> ActionResult:
        """Write content directly to markdown or PDF on the host."""
        if output_format == "pdf":
            return await self._generate_pdf(content, output_filename, step_dir)

        # Default: markdown
        if not output_filename.endswith(".md"):
            output_filename = output_filename.rsplit(".", 1)[0] + ".md" if "." in output_filename else output_filename + ".md"

        out_path = step_dir / output_filename
        out_path.write_text(content, encoding="utf-8")
        logger.info("Action: wrote markdown %s (%d bytes)", out_path, out_path.stat().st_size)

        return ActionResult(
            file_path=str(out_path),
            file_name=output_filename,
            mime_type="text/markdown",
            size_bytes=out_path.stat().st_size,
            output_format="markdown",
        )

    async def _generate_pdf(self, content: str, output_filename: str, step_dir: Path) -> ActionResult:
        """Convert text content to a PDF using fpdf2."""
        try:
            from fpdf import FPDF  # type: ignore[import-untyped]
        except ImportError as exc:
            raise RuntimeError("fpdf2 is required for PDF generation — pip install fpdf2") from exc

        if not output_filename.endswith(".pdf"):
            output_filename = output_filename.rsplit(".", 1)[0] + ".pdf" if "." in output_filename else output_filename + ".pdf"

        pdf = FPDF()
        pdf.set_auto_page_break(auto=True, margin=15)
        pdf.add_page()
        pdf.set_font("Helvetica", size=11)

        for line in content.split("\n"):
            # Basic markdown header detection for styling
            if line.startswith("# "):
                pdf.set_font("Helvetica", "B", 16)
                pdf.cell(0, 10, line[2:].strip(), new_x="LMARGIN", new_y="NEXT")
                pdf.set_font("Helvetica", size=11)
            elif line.startswith("## "):
                pdf.set_font("Helvetica", "B", 14)
                pdf.cell(0, 9, line[3:].strip(), new_x="LMARGIN", new_y="NEXT")
                pdf.set_font("Helvetica", size=11)
            elif line.startswith("### "):
                pdf.set_font("Helvetica", "B", 12)
                pdf.cell(0, 8, line[4:].strip(), new_x="LMARGIN", new_y="NEXT")
                pdf.set_font("Helvetica", size=11)
            elif line.strip() == "":
                pdf.ln(4)
            else:
                pdf.multi_cell(0, 6, line)

        out_path = step_dir / output_filename
        pdf.output(str(out_path))
        logger.info("Action: wrote PDF %s (%d bytes)", out_path, out_path.stat().st_size)

        return ActionResult(
            file_path=str(out_path),
            file_name=output_filename,
            mime_type="application/pdf",
            size_bytes=out_path.stat().st_size,
            output_format="pdf",
        )

    # ── Container-based transformer (MCP / executable) ────────────────────────

    async def _run_in_container(
        self,
        previous_output: str,
        action_config: dict[str, Any],
        output_format: str,
        output_filename: str,
        step_dir: Path,
    ) -> ActionResult:
        """Run a transformation script inside a Docker executor container."""
        transformer_type = action_config.get("transformerType", "mcp")

        # Resolve the script to execute
        script_path = await self._resolve_script(action_config, transformer_type)
        if not script_path or not script_path.exists():
            return ActionResult(
                file_path="",
                file_name=output_filename,
                mime_type="",
                size_bytes=0,
                output_format=output_format,
                success=False,
                error=f"Script not found for {transformer_type} transformer",
            )

        # Determine which container to use
        container, interpreter = self._resolve_container(script_path, action_config)

        # Write input to the shared mount (engine/steps/execution/<wf>/<step>/)
        input_file = step_dir / "action_input.txt"
        input_file.write_text(previous_output, encoding="utf-8")

        # Compute container-side paths via the /output mount
        rel_path = step_dir.relative_to(STEPS_EXECUTION_ROOT)
        container_input = f"/output/{rel_path}/action_input.txt"
        container_output = f"/output/{rel_path}/{output_filename}"

        # Copy script into container workspace
        script_dest = f"/workspace/action_transform{script_path.suffix}"
        await self._docker_cp(str(script_path), container, script_dest)

        # Execute inside container
        env_vars = [
            "-e",
            f"INPUT_FILE={container_input}",
            "-e",
            f"OUTPUT_FILE={container_output}",
            "-e",
            f"OUTPUT_FORMAT={output_format}",
        ]

        cmd = ["docker", "exec", *env_vars, container, *interpreter, script_dest]
        logger.info("Action: running in %s → %s", container, " ".join(cmd))

        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=120)
        except TimeoutError:
            proc.kill()
            return ActionResult(
                file_path="",
                file_name=output_filename,
                mime_type="",
                size_bytes=0,
                output_format=output_format,
                success=False,
                error="Container execution timed out after 120s",
            )

        if proc.returncode != 0:
            err_msg = stderr.decode("utf-8", errors="replace").strip()
            logger.error("Container script failed (rc=%d): %s", proc.returncode, err_msg)
            return ActionResult(
                file_path="",
                file_name=output_filename,
                mime_type="",
                size_bytes=0,
                output_format=output_format,
                success=False,
                error=f"Script exited with code {proc.returncode}: {err_msg[:500]}",
            )

        # Read output file from the shared mount on the host side
        host_output = step_dir / output_filename
        if not host_output.exists():
            # Maybe the script wrote to stdout instead — fall back
            stdout_text = stdout.decode("utf-8", errors="replace").strip()
            if stdout_text:
                host_output.write_text(stdout_text, encoding="utf-8")
                logger.info("Action: script wrote to stdout, saved as %s", host_output)
            else:
                return ActionResult(
                    file_path="",
                    file_name=output_filename,
                    mime_type="",
                    size_bytes=0,
                    output_format=output_format,
                    success=False,
                    error="Script completed but no output file was generated",
                )

        # Clean up input file
        input_file.unlink(missing_ok=True)

        mime = "application/pdf" if output_format == "pdf" else "text/markdown"
        size = host_output.stat().st_size
        logger.info("Action: container produced %s (%d bytes)", host_output, size)

        return ActionResult(
            file_path=str(host_output),
            file_name=output_filename,
            mime_type=mime,
            size_bytes=size,
            output_format=output_format,
        )

    # ── Helpers ───────────────────────────────────────────────────────────────

    async def _resolve_script(self, action_config: dict[str, Any], transformer_type: str) -> Path | None:
        """Locate the script file for an MCP or executable transformer."""
        if transformer_type == "mcp":
            mcp_server_id = action_config.get("mcpServerId", "")
            if not mcp_server_id:
                return None
            # Import here to avoid circular imports
            from engine.services.mcp import mcp_service

            # Check cache first, then prebuilt, then persisted
            cached = mcp_service.get_cached(mcp_server_id)
            if cached and cached.get("code"):
                # Write to a temp file
                ext = ".py" if cached.get("language") == "python" else ".ts" if cached.get("language") == "typescript" else ".js"
                tmp = Path(tempfile.gettempdir()) / f"mcp_action_{mcp_server_id}{ext}"
                tmp.write_text(cached["code"], encoding="utf-8")
                return tmp

            # Check prebuilt
            for server in mcp_service.list_prebuilt():
                if server["id"] == mcp_server_id:
                    from engine.services.mcp import MCP_PREBUILT_DIR

                    script_file = MCP_PREBUILT_DIR / server["filename"]
                    if script_file.exists():
                        return script_file

            # Check persisted
            for server in mcp_service.list_persisted():
                if server["id"] == mcp_server_id:
                    from engine.services.mcp import MCP_SCRIPTS_DIR

                    script_file = MCP_SCRIPTS_DIR / server["filename"]
                    if script_file.exists():
                        return script_file

            return None

        elif transformer_type == "executable":
            exec_path = action_config.get("executablePath", "")
            if not exec_path:
                return None
            p = Path(exec_path)
            return p if p.exists() else None

        return None

    def _resolve_container(self, script_path: Path, action_config: dict[str, Any]) -> tuple[str, list[str]]:
        """Determine which Docker container and interpreter to use."""
        # Explicit runtime override
        runtime = action_config.get("executionRuntime")
        if runtime == "python":
            return PYTHON_CONTAINER, ["python"]
        if runtime == "typescript":
            return TS_CONTAINER, ["npx", "tsx"]

        # Auto-detect from file extension
        ext = script_path.suffix.lower()
        if ext in _LANG_MAP:
            return _LANG_MAP[ext]

        # Fallback to Python container
        logger.warning("Unknown script extension %s, defaulting to Python container", ext)
        return PYTHON_CONTAINER, ["python"]

    async def _docker_cp(self, src: str, container: str, dest: str) -> None:
        """Copy a file into a Docker container."""
        proc = await asyncio.create_subprocess_exec(
            "docker",
            "cp",
            src,
            f"{container}:{dest}",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        _, stderr = await asyncio.wait_for(proc.communicate(), timeout=30)
        if proc.returncode != 0:
            raise RuntimeError(f"docker cp failed: {stderr.decode('utf-8', errors='replace').strip()}")


# Singleton
action_runner = ActionRunner()
