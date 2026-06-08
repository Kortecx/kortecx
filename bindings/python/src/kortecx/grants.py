"""UI-3 sharing (grants) views — every grant on an asset, fold-classified
root/delegated + active/revoked (``ListAssetGrants``).

Kept in its own module so ``types.py`` stays a thin aggregator. A VIEW-only surface;
issuing/revoking grants is cloud.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class GrantView:
    """One grant on an asset, fold-classified."""

    grantor: str
    grantee: str
    actions: List[str]
    runtime_scope: str
    is_root: bool  # a root grant (from the asset owner), vs a delegated sub-grant
    revoked: bool  # an authorized revocation makes the grant inert in the fold

    @classmethod
    def from_proto(cls, g: "_g.GrantView") -> "GrantView":
        return cls(
            grantor=g.grantor,
            grantee=g.grantee,
            actions=list(g.actions),
            runtime_scope=g.runtime_scope,
            is_root=g.is_root,
            revoked=g.revoked,
        )

    @property
    def status(self) -> str:
        """A stable display status: ``"revoked"`` | ``"root"`` | ``"delegated"``."""
        if self.revoked:
            return "revoked"
        return "root" if self.is_root else "delegated"


@dataclass(frozen=True)
class AssetGrants:
    """Every grant on one asset, with the bound owner echoed (``""`` if unbound)."""

    owner: str
    grants: List[GrantView]

    @classmethod
    def from_proto(cls, r: "_g.ListAssetGrantsResponse") -> "AssetGrants":
        return cls(
            owner=r.owner,
            grants=[GrantView.from_proto(g) for g in r.grants],
        )
