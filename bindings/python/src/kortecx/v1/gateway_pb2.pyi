from kortecx.v1 import coordinator_pb2 as _coordinator_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Iterable as _Iterable, Mapping as _Mapping, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class MoteSnapshotState(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MOTE_SNAPSHOT_STATE_UNSPECIFIED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_PENDING: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_SCHEDULED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_COMMITTED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_FAILED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_REPUDIATED: _ClassVar[MoteSnapshotState]
    MOTE_SNAPSHOT_STATE_INCONSISTENT: _ClassVar[MoteSnapshotState]

class PromotionState(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    PROMOTION_STATE_UNSPECIFIED: _ClassVar[PromotionState]
    PROMOTION_STATE_NOT_APPLICABLE: _ClassVar[PromotionState]
    PROMOTION_STATE_UNPROMOTED: _ClassVar[PromotionState]
    PROMOTION_STATE_PROMOTED: _ClassVar[PromotionState]

class MoteAnomaly(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MOTE_ANOMALY_UNSPECIFIED: _ClassVar[MoteAnomaly]
    MOTE_ANOMALY_EFFECT_STAGED_THEN_REPUDIATED_NO_COMMITTED: _ClassVar[MoteAnomaly]
    MOTE_ANOMALY_QUARANTINED_AT_LEAST_ONCE_EFFECT: _ClassVar[MoteAnomaly]
MOTE_SNAPSHOT_STATE_UNSPECIFIED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_PENDING: MoteSnapshotState
MOTE_SNAPSHOT_STATE_SCHEDULED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_COMMITTED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_FAILED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_REPUDIATED: MoteSnapshotState
MOTE_SNAPSHOT_STATE_INCONSISTENT: MoteSnapshotState
PROMOTION_STATE_UNSPECIFIED: PromotionState
PROMOTION_STATE_NOT_APPLICABLE: PromotionState
PROMOTION_STATE_UNPROMOTED: PromotionState
PROMOTION_STATE_PROMOTED: PromotionState
MOTE_ANOMALY_UNSPECIFIED: MoteAnomaly
MOTE_ANOMALY_EFFECT_STAGED_THEN_REPUDIATED_NO_COMMITTED: MoteAnomaly
MOTE_ANOMALY_QUARANTINED_AT_LEAST_ONCE_EFFECT: MoteAnomaly

class SubmitRunRequest(_message.Message):
    __slots__ = ("recipe_fingerprint", "motes")
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    MOTES_FIELD_NUMBER: _ClassVar[int]
    recipe_fingerprint: bytes
    motes: _containers.RepeatedCompositeFieldContainer[SubmitMoteSpec]
    def __init__(self, recipe_fingerprint: _Optional[bytes] = ..., motes: _Optional[_Iterable[_Union[SubmitMoteSpec, _Mapping]]] = ...) -> None: ...

class SubmitMoteSpec(_message.Message):
    __slots__ = ("mote", "warrant", "accept_at_least_once")
    MOTE_FIELD_NUMBER: _ClassVar[int]
    WARRANT_FIELD_NUMBER: _ClassVar[int]
    ACCEPT_AT_LEAST_ONCE_FIELD_NUMBER: _ClassVar[int]
    mote: _coordinator_pb2.Mote
    warrant: _coordinator_pb2.WarrantSpec
    accept_at_least_once: bool
    def __init__(self, mote: _Optional[_Union[_coordinator_pb2.Mote, _Mapping]] = ..., warrant: _Optional[_Union[_coordinator_pb2.WarrantSpec, _Mapping]] = ..., accept_at_least_once: bool = ...) -> None: ...

class RunHandle(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ...) -> None: ...

class InvokeRequest(_message.Message):
    __slots__ = ("handle", "args")
    HANDLE_FIELD_NUMBER: _ClassVar[int]
    ARGS_FIELD_NUMBER: _ClassVar[int]
    handle: str
    args: bytes
    def __init__(self, handle: _Optional[str] = ..., args: _Optional[bytes] = ...) -> None: ...

class InvokeResponse(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint", "terminal_mote_id")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    TERMINAL_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    terminal_mote_id: bytes
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ..., terminal_mote_id: _Optional[bytes] = ...) -> None: ...

class GetProjectionRequest(_message.Message):
    __slots__ = ("instance_id", "at_seq")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    AT_SEQ_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    at_seq: int
    def __init__(self, instance_id: _Optional[bytes] = ..., at_seq: _Optional[int] = ...) -> None: ...

class ProjectionView(_message.Message):
    __slots__ = ("instance_id", "recipe_fingerprint", "current_seq", "motes")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    CURRENT_SEQ_FIELD_NUMBER: _ClassVar[int]
    MOTES_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    recipe_fingerprint: bytes
    current_seq: int
    motes: _containers.RepeatedCompositeFieldContainer[MoteSnapshot]
    def __init__(self, instance_id: _Optional[bytes] = ..., recipe_fingerprint: _Optional[bytes] = ..., current_seq: _Optional[int] = ..., motes: _Optional[_Iterable[_Union[MoteSnapshot, _Mapping]]] = ...) -> None: ...

class MoteSnapshot(_message.Message):
    __slots__ = ("mote_id", "state", "nd_class", "promotion", "result_ref", "warrant_ref", "mote_def_hash", "committed_seq", "parents", "verdict", "anomaly")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    STATE_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    PROMOTION_FIELD_NUMBER: _ClassVar[int]
    RESULT_REF_FIELD_NUMBER: _ClassVar[int]
    WARRANT_REF_FIELD_NUMBER: _ClassVar[int]
    MOTE_DEF_HASH_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_SEQ_FIELD_NUMBER: _ClassVar[int]
    PARENTS_FIELD_NUMBER: _ClassVar[int]
    VERDICT_FIELD_NUMBER: _ClassVar[int]
    ANOMALY_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    state: MoteSnapshotState
    nd_class: _coordinator_pb2.NdClass
    promotion: PromotionState
    result_ref: bytes
    warrant_ref: bytes
    mote_def_hash: bytes
    committed_seq: int
    parents: _containers.RepeatedCompositeFieldContainer[_coordinator_pb2.ParentRef]
    verdict: bytes
    anomaly: MoteAnomaly
    def __init__(self, mote_id: _Optional[bytes] = ..., state: _Optional[_Union[MoteSnapshotState, str]] = ..., nd_class: _Optional[_Union[_coordinator_pb2.NdClass, str]] = ..., promotion: _Optional[_Union[PromotionState, str]] = ..., result_ref: _Optional[bytes] = ..., warrant_ref: _Optional[bytes] = ..., mote_def_hash: _Optional[bytes] = ..., committed_seq: _Optional[int] = ..., parents: _Optional[_Iterable[_Union[_coordinator_pb2.ParentRef, _Mapping]]] = ..., verdict: _Optional[bytes] = ..., anomaly: _Optional[_Union[MoteAnomaly, str]] = ...) -> None: ...

class GetContentRequest(_message.Message):
    __slots__ = ("content_ref", "instance_id")
    CONTENT_REF_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    content_ref: bytes
    instance_id: bytes
    def __init__(self, content_ref: _Optional[bytes] = ..., instance_id: _Optional[bytes] = ...) -> None: ...

class ContentBlob(_message.Message):
    __slots__ = ("payload",)
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    payload: bytes
    def __init__(self, payload: _Optional[bytes] = ...) -> None: ...

class StreamEventsRequest(_message.Message):
    __slots__ = ("instance_id", "since_seq")
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    SINCE_SEQ_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    since_seq: int
    def __init__(self, instance_id: _Optional[bytes] = ..., since_seq: _Optional[int] = ...) -> None: ...

class EventFrame(_message.Message):
    __slots__ = ("seq", "deltas", "next_seq", "journal_boundary")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    DELTAS_FIELD_NUMBER: _ClassVar[int]
    NEXT_SEQ_FIELD_NUMBER: _ClassVar[int]
    JOURNAL_BOUNDARY_FIELD_NUMBER: _ClassVar[int]
    seq: int
    deltas: _containers.RepeatedCompositeFieldContainer[EventDelta]
    next_seq: int
    journal_boundary: bool
    def __init__(self, seq: _Optional[int] = ..., deltas: _Optional[_Iterable[_Union[EventDelta, _Mapping]]] = ..., next_seq: _Optional[int] = ..., journal_boundary: bool = ...) -> None: ...

class EventDelta(_message.Message):
    __slots__ = ("seq", "committed", "failed", "repudiated", "effect_staged")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_FIELD_NUMBER: _ClassVar[int]
    FAILED_FIELD_NUMBER: _ClassVar[int]
    REPUDIATED_FIELD_NUMBER: _ClassVar[int]
    EFFECT_STAGED_FIELD_NUMBER: _ClassVar[int]
    seq: int
    committed: CommittedDelta
    failed: FailedDelta
    repudiated: RepudiatedDelta
    effect_staged: EffectStagedDelta
    def __init__(self, seq: _Optional[int] = ..., committed: _Optional[_Union[CommittedDelta, _Mapping]] = ..., failed: _Optional[_Union[FailedDelta, _Mapping]] = ..., repudiated: _Optional[_Union[RepudiatedDelta, _Mapping]] = ..., effect_staged: _Optional[_Union[EffectStagedDelta, _Mapping]] = ...) -> None: ...

class CommittedDelta(_message.Message):
    __slots__ = ("mote_id", "result_ref", "nd_class")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    RESULT_REF_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    result_ref: bytes
    nd_class: _coordinator_pb2.NdClass
    def __init__(self, mote_id: _Optional[bytes] = ..., result_ref: _Optional[bytes] = ..., nd_class: _Optional[_Union[_coordinator_pb2.NdClass, str]] = ...) -> None: ...

class FailedDelta(_message.Message):
    __slots__ = ("mote_id", "reason_class")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    REASON_CLASS_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    reason_class: int
    def __init__(self, mote_id: _Optional[bytes] = ..., reason_class: _Optional[int] = ...) -> None: ...

class RepudiatedDelta(_message.Message):
    __slots__ = ("target_mote_id", "target_committed_seq")
    TARGET_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    TARGET_COMMITTED_SEQ_FIELD_NUMBER: _ClassVar[int]
    target_mote_id: bytes
    target_committed_seq: int
    def __init__(self, target_mote_id: _Optional[bytes] = ..., target_committed_seq: _Optional[int] = ...) -> None: ...

class EffectStagedDelta(_message.Message):
    __slots__ = ("mote_id",)
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    def __init__(self, mote_id: _Optional[bytes] = ...) -> None: ...

class ListSignaturesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class SignatureSummary(_message.Message):
    __slots__ = ("signature_id", "name")
    SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    signature_id: bytes
    name: str
    def __init__(self, signature_id: _Optional[bytes] = ..., name: _Optional[str] = ...) -> None: ...

class ListSignaturesResponse(_message.Message):
    __slots__ = ("signatures",)
    SIGNATURES_FIELD_NUMBER: _ClassVar[int]
    signatures: _containers.RepeatedCompositeFieldContainer[SignatureSummary]
    def __init__(self, signatures: _Optional[_Iterable[_Union[SignatureSummary, _Mapping]]] = ...) -> None: ...

class GetSignatureRequest(_message.Message):
    __slots__ = ("signature_id",)
    SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    signature_id: bytes
    def __init__(self, signature_id: _Optional[bytes] = ...) -> None: ...

class GetSignatureResponse(_message.Message):
    __slots__ = ("signature_id", "manifest")
    SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    MANIFEST_FIELD_NUMBER: _ClassVar[int]
    signature_id: bytes
    manifest: bytes
    def __init__(self, signature_id: _Optional[bytes] = ..., manifest: _Optional[bytes] = ...) -> None: ...

class RegisterSignatureRequest(_message.Message):
    __slots__ = ("manifest",)
    MANIFEST_FIELD_NUMBER: _ClassVar[int]
    manifest: bytes
    def __init__(self, manifest: _Optional[bytes] = ...) -> None: ...

class RegisterSignatureResponse(_message.Message):
    __slots__ = ("signature_id",)
    SIGNATURE_ID_FIELD_NUMBER: _ClassVar[int]
    signature_id: bytes
    def __init__(self, signature_id: _Optional[bytes] = ...) -> None: ...
