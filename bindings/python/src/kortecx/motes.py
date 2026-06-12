"""Batch B per-mote definition view — ``GetMoteDetail``. DISPLAY-ONLY (SN-8):
the capped definition summary the coordinator persisted at admission, resolved
by ``mote_def_hash``; nothing here authorizes anything. A mote that has not
committed (or was admitted by a pre-Batch-B binary) answers
``def_found=False`` honestly. Kept in its own module per the
module-per-concern discipline (the ``runs.py`` pattern).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, List, Optional

from . import hexids
from .v1 import gateway_pb2 as _g

_ND_CLASS_NAMES = {1: "PURE", 2: "READ_ONLY_NONDET", 3: "WORLD_MUTATING"}
_EFFECT_PATTERN_NAMES = {
    1: "IdempotentByConstruction",
    2: "StageThenCommit",
    3: "ValidateThenCommit",
}


def nd_class_name(nd: int) -> str:
    """Display name for a wire ``NdClass`` discriminant (CLI-parity strings)."""
    return _ND_CLASS_NAMES.get(nd, "UNKNOWN")


def effect_pattern_name(ep: int) -> str:
    """Display name for a wire ``EffectPattern`` discriminant (CLI-parity strings)."""
    return _EFFECT_PATTERN_NAMES.get(ep, "UNKNOWN")


@dataclass(frozen=True)
class MoteConfigItem:
    """One capped config entry of a Mote definition (opaque display bytes)."""

    key: str
    value: bytes  # possibly truncated
    truncated: bool
    full_len: int

    @classmethod
    def from_proto(cls, e: "_g.MoteConfigEntry") -> "MoteConfigItem":
        return cls(key=e.key, value=e.value, truncated=e.truncated, full_len=e.full_len)

    def to_dict(self) -> dict:
        """The CLI ``--json`` parity shape."""
        return {
            "key": self.key,
            "value_hex": self.value.hex(),
            "truncated": self.truncated,
            "full_len": self.full_len,
        }


@dataclass(frozen=True)
class MoteDetail:
    """The capped, display-only definition summary of one Mote."""

    mote_id: str  # hex
    mote_def_hash: str  # hex; EMPTY until the Mote commits
    def_found: bool  # False: uncommitted, or admitted by a pre-Batch-B binary
    step_kind: str  # "pure"|"model"|"exec"|"shaper"|"critic"|"react-turn" (display)
    model_id: str
    prompt: str  # config_subset["prompt"], capped server-side
    prompt_truncated: bool
    config_subset: List[MoteConfigItem]
    tool_contract: Dict[str, str]  # tool name -> pinned version
    logic_ref: str  # hex
    nd_class: int
    effect_pattern: int
    critic_for: Optional[str]  # hex producer id, or None
    is_topology_shaper: bool
    schema_version: int

    @classmethod
    def from_proto(cls, d: "_g.MoteDetail") -> "MoteDetail":
        return cls(
            mote_id=hexids.encode(d.mote_id),
            mote_def_hash=hexids.encode(d.mote_def_hash),
            def_found=d.def_found,
            step_kind=d.step_kind,
            model_id=d.model_id,
            prompt=d.prompt,
            prompt_truncated=d.prompt_truncated,
            config_subset=[MoteConfigItem.from_proto(e) for e in d.config_subset],
            tool_contract=dict(d.tool_contract),
            logic_ref=hexids.encode(d.logic_ref),
            nd_class=d.nd_class,
            effect_pattern=d.effect_pattern,
            critic_for=hexids.encode(d.critic_for) if d.HasField("critic_for") else None,
            is_topology_shaper=d.is_topology_shaper,
            schema_version=d.schema_version,
        )

    @property
    def nd_class_name(self) -> str:
        """Display name for :attr:`nd_class`."""
        return nd_class_name(self.nd_class)

    @property
    def effect_pattern_name(self) -> str:
        """Display name for :attr:`effect_pattern`."""
        return effect_pattern_name(self.effect_pattern)

    def to_dict(self) -> dict:
        """Field-for-field the CLI ``--json`` shape (the tri-surface parity
        contract — byte-identical to the TS ``MoteDetail.toJSON()``)."""
        return {
            "mote_id": self.mote_id,
            "mote_def_hash": self.mote_def_hash,
            "def_found": self.def_found,
            "step_kind": self.step_kind,
            "model_id": self.model_id,
            "prompt": self.prompt,
            "prompt_truncated": self.prompt_truncated,
            "config_subset": [e.to_dict() for e in self.config_subset],
            "tool_contract": dict(sorted(self.tool_contract.items())),
            "logic_ref": self.logic_ref,
            "nd_class": self.nd_class_name,
            "effect_pattern": self.effect_pattern_name,
            "critic_for": self.critic_for,
            "is_topology_shaper": self.is_topology_shaper,
            "schema_version": self.schema_version,
        }
