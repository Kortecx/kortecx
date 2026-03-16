"""Cross-platform system resource monitoring — CPU, GPU, memory."""

from __future__ import annotations

import logging
import platform
from typing import Any

import psutil

logger = logging.getLogger("engine.system_stats")


def get_system_stats() -> dict[str, Any]:
    """Get current CPU, memory, and GPU usage. Works on macOS, Linux, and Windows."""
    stats: dict[str, Any] = {
        "platform": platform.system(),
        "cpu_percent": psutil.cpu_percent(interval=0.1),
        "cpu_count": psutil.cpu_count(),
        "cpu_freq_mhz": None,
        "memory_percent": psutil.virtual_memory().percent,
        "memory_used_gb": round(psutil.virtual_memory().used / (1024 ** 3), 1),
        "memory_total_gb": round(psutil.virtual_memory().total / (1024 ** 3), 1),
        "gpu": None,
    }

    # CPU frequency
    freq = psutil.cpu_freq()
    if freq:
        stats["cpu_freq_mhz"] = round(freq.current)

    # GPU detection
    stats["gpu"] = _get_gpu_stats()

    return stats


def _get_gpu_stats() -> dict[str, Any] | None:
    """Detect GPU usage — supports NVIDIA (nvidia-smi), Apple Silicon (powermetrics), and basic fallback."""

    # NVIDIA GPU via pynvml
    try:
        import pynvml  # type: ignore[import-untyped]
        pynvml.nvmlInit()
        count = pynvml.nvmlDeviceGetCount()
        gpus = []
        for i in range(count):
            handle = pynvml.nvmlDeviceGetHandleByIndex(i)
            name = pynvml.nvmlDeviceGetName(handle)
            if isinstance(name, bytes):
                name = name.decode()
            util = pynvml.nvmlDeviceGetUtilizationRates(handle)
            mem = pynvml.nvmlDeviceGetMemoryInfo(handle)
            gpus.append({
                "name": name,
                "gpu_percent": util.gpu,
                "memory_percent": round(mem.used / mem.total * 100, 1) if mem.total > 0 else 0,
                "memory_used_mb": round(mem.used / (1024 ** 2)),
                "memory_total_mb": round(mem.total / (1024 ** 2)),
            })
        pynvml.nvmlShutdown()
        if gpus:
            return {"type": "nvidia", "devices": gpus}
    except Exception:
        pass

    # Apple Silicon — estimate from process CPU (Metal doesn't expose GPU% easily)
    if platform.system() == "Darwin" and platform.machine() == "arm64":
        return {
            "type": "apple_silicon",
            "devices": [{
                "name": f"Apple {platform.machine().upper()} GPU",
                "gpu_percent": None,  # Not directly available without powermetrics (needs sudo)
                "note": "Apple Silicon — GPU shares unified memory with CPU",
            }],
        }

    return None


def get_process_stats(pid: int | None = None) -> dict[str, Any]:
    """Get resource usage for a specific process (or current process)."""
    import os
    try:
        p = psutil.Process(pid or os.getpid())
        with p.oneshot():
            return {
                "pid": p.pid,
                "cpu_percent": p.cpu_percent(interval=0.1),
                "memory_mb": round(p.memory_info().rss / (1024 ** 2), 1),
                "memory_percent": round(p.memory_percent(), 1),
                "threads": p.num_threads(),
            }
    except Exception:
        return {"pid": pid, "cpu_percent": 0, "memory_mb": 0, "memory_percent": 0, "threads": 0}
