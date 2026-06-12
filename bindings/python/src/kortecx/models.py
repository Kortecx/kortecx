"""Batch A model-discovery view — one ``ListModels`` entry.

Display/discovery ONLY (SN-8): model *selection* stays a recipe ENUM free-param
validated server-side at binding; nothing here authorizes a model route. An
FFI-free gateway answers with an EMPTY list (honest, not an error).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Tuple

from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class ModelSummary:
    """One discoverable model on the connected gateway."""

    model_id: str  # the id a recipe `model` ENUM free-param accepts
    modalities: Tuple[str, ...]  # display strings: "text" | "image" | "audio" | "video"
    description: str  # host-synthesized display prose — never identity
    serving: bool  # this model backs the live serve loop right now
    context_len: int  # the served context window in tokens

    @classmethod
    def from_proto(cls, m: "_g.ModelSummary") -> "ModelSummary":
        return cls(
            model_id=m.model_id,
            modalities=tuple(m.modalities),
            description=m.description,
            serving=m.serving,
            context_len=m.context_len,
        )
