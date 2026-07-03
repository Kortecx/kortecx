from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Iterable as _Iterable, Mapping as _Mapping, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class NdClass(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    ND_CLASS_UNSPECIFIED: _ClassVar[NdClass]
    ND_CLASS_PURE: _ClassVar[NdClass]
    ND_CLASS_READ_ONLY_NONDET: _ClassVar[NdClass]
    ND_CLASS_WORLD_MUTATING: _ClassVar[NdClass]

class EffectPattern(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    EFFECT_PATTERN_UNSPECIFIED: _ClassVar[EffectPattern]
    EFFECT_PATTERN_IDEMPOTENT_BY_CONSTRUCTION: _ClassVar[EffectPattern]
    EFFECT_PATTERN_STAGE_THEN_COMMIT: _ClassVar[EffectPattern]
    EFFECT_PATTERN_VALIDATE_THEN_COMMIT: _ClassVar[EffectPattern]

class EdgeKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    EDGE_KIND_UNSPECIFIED: _ClassVar[EdgeKind]
    EDGE_KIND_DATA: _ClassVar[EdgeKind]
    EDGE_KIND_CONTROL: _ClassVar[EdgeKind]

class MoteClass(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MOTE_CLASS_UNSPECIFIED: _ClassVar[MoteClass]
    MOTE_CLASS_PURE: _ClassVar[MoteClass]
    MOTE_CLASS_READ_ONLY_NONDET: _ClassVar[MoteClass]
    MOTE_CLASS_WORLD_MUTATING: _ClassVar[MoteClass]

class FsMode(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    FS_MODE_UNSPECIFIED: _ClassVar[FsMode]
    FS_MODE_READ_ONLY: _ClassVar[FsMode]
    FS_MODE_READ_WRITE: _ClassVar[FsMode]
    FS_MODE_EXEC_ONLY: _ClassVar[FsMode]

class FailureReason(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    FAILURE_REASON_UNSPECIFIED: _ClassVar[FailureReason]
    FAILURE_REASON_TIMED_OUT: _ClassVar[FailureReason]
    FAILURE_REASON_EXECUTOR_REFUSED: _ClassVar[FailureReason]
    FAILURE_REASON_VALIDATOR_REJECTED: _ClassVar[FailureReason]
    FAILURE_REASON_WORKER_CRASHED: _ClassVar[FailureReason]
    FAILURE_REASON_UPSTREAM_REPUDIATED: _ClassVar[FailureReason]
    FAILURE_REASON_UNSAFE_WORLD_MUTATING_CONSTRUCTION: _ClassVar[FailureReason]
    FAILURE_REASON_COMPENSATED_AT_LEAST_ONCE: _ClassVar[FailureReason]
    FAILURE_REASON_QUARANTINED_AT_LEAST_ONCE: _ClassVar[FailureReason]
    FAILURE_REASON_DEAD_LETTERED: _ClassVar[FailureReason]

class ExecutorClass(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    EXECUTOR_CLASS_UNSPECIFIED: _ClassVar[ExecutorClass]
    EXECUTOR_CLASS_BWRAP: _ClassVar[ExecutorClass]
    EXECUTOR_CLASS_OCI_DAEMON: _ClassVar[ExecutorClass]
    EXECUTOR_CLASS_CLOUD_MICRO_VM: _ClassVar[ExecutorClass]
    EXECUTOR_CLASS_MACOS_SANDBOX: _ClassVar[ExecutorClass]

class SubmitStatus(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    SUBMIT_STATUS_UNSPECIFIED: _ClassVar[SubmitStatus]
    SUBMIT_STATUS_ACCEPTED: _ClassVar[SubmitStatus]
    SUBMIT_STATUS_DUPLICATE: _ClassVar[SubmitStatus]
    SUBMIT_STATUS_REJECTED: _ClassVar[SubmitStatus]

class CommitOutcome(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    COMMIT_OUTCOME_UNSPECIFIED: _ClassVar[CommitOutcome]
    COMMIT_OUTCOME_COMMITTED: _ClassVar[CommitOutcome]
    COMMIT_OUTCOME_ALREADY_COMMITTED: _ClassVar[CommitOutcome]
    COMMIT_OUTCOME_REJECTED: _ClassVar[CommitOutcome]
ND_CLASS_UNSPECIFIED: NdClass
ND_CLASS_PURE: NdClass
ND_CLASS_READ_ONLY_NONDET: NdClass
ND_CLASS_WORLD_MUTATING: NdClass
EFFECT_PATTERN_UNSPECIFIED: EffectPattern
EFFECT_PATTERN_IDEMPOTENT_BY_CONSTRUCTION: EffectPattern
EFFECT_PATTERN_STAGE_THEN_COMMIT: EffectPattern
EFFECT_PATTERN_VALIDATE_THEN_COMMIT: EffectPattern
EDGE_KIND_UNSPECIFIED: EdgeKind
EDGE_KIND_DATA: EdgeKind
EDGE_KIND_CONTROL: EdgeKind
MOTE_CLASS_UNSPECIFIED: MoteClass
MOTE_CLASS_PURE: MoteClass
MOTE_CLASS_READ_ONLY_NONDET: MoteClass
MOTE_CLASS_WORLD_MUTATING: MoteClass
FS_MODE_UNSPECIFIED: FsMode
FS_MODE_READ_ONLY: FsMode
FS_MODE_READ_WRITE: FsMode
FS_MODE_EXEC_ONLY: FsMode
FAILURE_REASON_UNSPECIFIED: FailureReason
FAILURE_REASON_TIMED_OUT: FailureReason
FAILURE_REASON_EXECUTOR_REFUSED: FailureReason
FAILURE_REASON_VALIDATOR_REJECTED: FailureReason
FAILURE_REASON_WORKER_CRASHED: FailureReason
FAILURE_REASON_UPSTREAM_REPUDIATED: FailureReason
FAILURE_REASON_UNSAFE_WORLD_MUTATING_CONSTRUCTION: FailureReason
FAILURE_REASON_COMPENSATED_AT_LEAST_ONCE: FailureReason
FAILURE_REASON_QUARANTINED_AT_LEAST_ONCE: FailureReason
FAILURE_REASON_DEAD_LETTERED: FailureReason
EXECUTOR_CLASS_UNSPECIFIED: ExecutorClass
EXECUTOR_CLASS_BWRAP: ExecutorClass
EXECUTOR_CLASS_OCI_DAEMON: ExecutorClass
EXECUTOR_CLASS_CLOUD_MICRO_VM: ExecutorClass
EXECUTOR_CLASS_MACOS_SANDBOX: ExecutorClass
SUBMIT_STATUS_UNSPECIFIED: SubmitStatus
SUBMIT_STATUS_ACCEPTED: SubmitStatus
SUBMIT_STATUS_DUPLICATE: SubmitStatus
SUBMIT_STATUS_REJECTED: SubmitStatus
COMMIT_OUTCOME_UNSPECIFIED: CommitOutcome
COMMIT_OUTCOME_COMMITTED: CommitOutcome
COMMIT_OUTCOME_ALREADY_COMMITTED: CommitOutcome
COMMIT_OUTCOME_REJECTED: CommitOutcome

class InferenceParams(_message.Message):
    __slots__ = ("max_output_tokens", "temperature_bps", "top_p_bps", "top_k", "seed", "stop_tokens", "grammar")
    MAX_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    TEMPERATURE_BPS_FIELD_NUMBER: _ClassVar[int]
    TOP_P_BPS_FIELD_NUMBER: _ClassVar[int]
    TOP_K_FIELD_NUMBER: _ClassVar[int]
    SEED_FIELD_NUMBER: _ClassVar[int]
    STOP_TOKENS_FIELD_NUMBER: _ClassVar[int]
    GRAMMAR_FIELD_NUMBER: _ClassVar[int]
    max_output_tokens: int
    temperature_bps: int
    top_p_bps: int
    top_k: int
    seed: int
    stop_tokens: _containers.RepeatedScalarFieldContainer[str]
    grammar: str
    def __init__(self, max_output_tokens: _Optional[int] = ..., temperature_bps: _Optional[int] = ..., top_p_bps: _Optional[int] = ..., top_k: _Optional[int] = ..., seed: _Optional[int] = ..., stop_tokens: _Optional[_Iterable[str]] = ..., grammar: _Optional[str] = ...) -> None: ...

class MoteDef(_message.Message):
    __slots__ = ("logic_ref", "model_id", "prompt_template_hash", "tool_contract", "nd_class", "config_subset", "effect_pattern", "critic_for", "is_topology_shaper", "inference_params", "schema_version", "critic_check")
    class ToolContractEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: str
        def __init__(self, key: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...
    class ConfigSubsetEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: bytes
        def __init__(self, key: _Optional[str] = ..., value: _Optional[bytes] = ...) -> None: ...
    LOGIC_REF_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    PROMPT_TEMPLATE_HASH_FIELD_NUMBER: _ClassVar[int]
    TOOL_CONTRACT_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    CONFIG_SUBSET_FIELD_NUMBER: _ClassVar[int]
    EFFECT_PATTERN_FIELD_NUMBER: _ClassVar[int]
    CRITIC_FOR_FIELD_NUMBER: _ClassVar[int]
    IS_TOPOLOGY_SHAPER_FIELD_NUMBER: _ClassVar[int]
    INFERENCE_PARAMS_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_VERSION_FIELD_NUMBER: _ClassVar[int]
    CRITIC_CHECK_FIELD_NUMBER: _ClassVar[int]
    logic_ref: bytes
    model_id: str
    prompt_template_hash: bytes
    tool_contract: _containers.ScalarMap[str, str]
    nd_class: NdClass
    config_subset: _containers.ScalarMap[str, bytes]
    effect_pattern: EffectPattern
    critic_for: bytes
    is_topology_shaper: bool
    inference_params: InferenceParams
    schema_version: int
    critic_check: bytes
    def __init__(self, logic_ref: _Optional[bytes] = ..., model_id: _Optional[str] = ..., prompt_template_hash: _Optional[bytes] = ..., tool_contract: _Optional[_Mapping[str, str]] = ..., nd_class: _Optional[_Union[NdClass, str]] = ..., config_subset: _Optional[_Mapping[str, bytes]] = ..., effect_pattern: _Optional[_Union[EffectPattern, str]] = ..., critic_for: _Optional[bytes] = ..., is_topology_shaper: bool = ..., inference_params: _Optional[_Union[InferenceParams, _Mapping]] = ..., schema_version: _Optional[int] = ..., critic_check: _Optional[bytes] = ...) -> None: ...

class ParentRef(_message.Message):
    __slots__ = ("parent_id", "edge_kind", "non_cascade")
    PARENT_ID_FIELD_NUMBER: _ClassVar[int]
    EDGE_KIND_FIELD_NUMBER: _ClassVar[int]
    NON_CASCADE_FIELD_NUMBER: _ClassVar[int]
    parent_id: bytes
    edge_kind: EdgeKind
    non_cascade: bool
    def __init__(self, parent_id: _Optional[bytes] = ..., edge_kind: _Optional[_Union[EdgeKind, str]] = ..., non_cascade: bool = ...) -> None: ...

class Mote(_message.Message):
    __slots__ = ("mote_id", "input_data_id", "graph_position", "parents")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    DEF_FIELD_NUMBER: _ClassVar[int]
    INPUT_DATA_ID_FIELD_NUMBER: _ClassVar[int]
    GRAPH_POSITION_FIELD_NUMBER: _ClassVar[int]
    PARENTS_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    input_data_id: bytes
    graph_position: bytes
    parents: _containers.RepeatedCompositeFieldContainer[ParentRef]
    def __init__(self, mote_id: _Optional[bytes] = ..., input_data_id: _Optional[bytes] = ..., graph_position: _Optional[bytes] = ..., parents: _Optional[_Iterable[_Union[ParentRef, _Mapping]]] = ..., **kwargs) -> None: ...

class ToolGrant(_message.Message):
    __slots__ = ("tool_id", "tool_version")
    TOOL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    tool_id: str
    tool_version: str
    def __init__(self, tool_id: _Optional[str] = ..., tool_version: _Optional[str] = ...) -> None: ...

class ModelRoute(_message.Message):
    __slots__ = ("model_id", "max_input_tokens", "max_output_tokens", "max_calls")
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    MAX_INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAX_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAX_CALLS_FIELD_NUMBER: _ClassVar[int]
    model_id: str
    max_input_tokens: int
    max_output_tokens: int
    max_calls: int
    def __init__(self, model_id: _Optional[str] = ..., max_input_tokens: _Optional[int] = ..., max_output_tokens: _Optional[int] = ..., max_calls: _Optional[int] = ...) -> None: ...

class ResourceCeiling(_message.Message):
    __slots__ = ("cpu_milli", "mem_bytes", "wall_clock_ms", "fd_count", "disk_bytes")
    CPU_MILLI_FIELD_NUMBER: _ClassVar[int]
    MEM_BYTES_FIELD_NUMBER: _ClassVar[int]
    WALL_CLOCK_MS_FIELD_NUMBER: _ClassVar[int]
    FD_COUNT_FIELD_NUMBER: _ClassVar[int]
    DISK_BYTES_FIELD_NUMBER: _ClassVar[int]
    cpu_milli: int
    mem_bytes: int
    wall_clock_ms: int
    fd_count: int
    disk_bytes: int
    def __init__(self, cpu_milli: _Optional[int] = ..., mem_bytes: _Optional[int] = ..., wall_clock_ms: _Optional[int] = ..., fd_count: _Optional[int] = ..., disk_bytes: _Optional[int] = ...) -> None: ...

class FsMount(_message.Message):
    __slots__ = ("path", "mode")
    PATH_FIELD_NUMBER: _ClassVar[int]
    MODE_FIELD_NUMBER: _ClassVar[int]
    path: str
    mode: FsMode
    def __init__(self, path: _Optional[str] = ..., mode: _Optional[_Union[FsMode, str]] = ...) -> None: ...

class FsScope(_message.Message):
    __slots__ = ("mounts",)
    MOUNTS_FIELD_NUMBER: _ClassVar[int]
    mounts: _containers.RepeatedCompositeFieldContainer[FsMount]
    def __init__(self, mounts: _Optional[_Iterable[_Union[FsMount, _Mapping]]] = ...) -> None: ...

class NetScope(_message.Message):
    __slots__ = ("none", "allowlist")
    NONE_FIELD_NUMBER: _ClassVar[int]
    ALLOWLIST_FIELD_NUMBER: _ClassVar[int]
    none: NetScopeNone
    allowlist: HostAllowlist
    def __init__(self, none: _Optional[_Union[NetScopeNone, _Mapping]] = ..., allowlist: _Optional[_Union[HostAllowlist, _Mapping]] = ...) -> None: ...

class NetScopeNone(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class HostAllowlist(_message.Message):
    __slots__ = ("hosts",)
    HOSTS_FIELD_NUMBER: _ClassVar[int]
    hosts: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, hosts: _Optional[_Iterable[str]] = ...) -> None: ...

class WarrantSpec(_message.Message):
    __slots__ = ("mote_class", "nd_class", "fs_scope", "net_scope", "syscall_profile_ref", "tool_grants", "model_route", "resource_ceiling", "environment_ref", "executor_class", "secret_scope")
    MOTE_CLASS_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    FS_SCOPE_FIELD_NUMBER: _ClassVar[int]
    NET_SCOPE_FIELD_NUMBER: _ClassVar[int]
    SYSCALL_PROFILE_REF_FIELD_NUMBER: _ClassVar[int]
    TOOL_GRANTS_FIELD_NUMBER: _ClassVar[int]
    MODEL_ROUTE_FIELD_NUMBER: _ClassVar[int]
    RESOURCE_CEILING_FIELD_NUMBER: _ClassVar[int]
    ENVIRONMENT_REF_FIELD_NUMBER: _ClassVar[int]
    EXECUTOR_CLASS_FIELD_NUMBER: _ClassVar[int]
    SECRET_SCOPE_FIELD_NUMBER: _ClassVar[int]
    mote_class: MoteClass
    nd_class: MoteClass
    fs_scope: FsScope
    net_scope: NetScope
    syscall_profile_ref: bytes
    tool_grants: _containers.RepeatedCompositeFieldContainer[ToolGrant]
    model_route: ModelRoute
    resource_ceiling: ResourceCeiling
    environment_ref: bytes
    executor_class: ExecutorClass
    secret_scope: SecretScope
    def __init__(self, mote_class: _Optional[_Union[MoteClass, str]] = ..., nd_class: _Optional[_Union[MoteClass, str]] = ..., fs_scope: _Optional[_Union[FsScope, _Mapping]] = ..., net_scope: _Optional[_Union[NetScope, _Mapping]] = ..., syscall_profile_ref: _Optional[bytes] = ..., tool_grants: _Optional[_Iterable[_Union[ToolGrant, _Mapping]]] = ..., model_route: _Optional[_Union[ModelRoute, _Mapping]] = ..., resource_ceiling: _Optional[_Union[ResourceCeiling, _Mapping]] = ..., environment_ref: _Optional[bytes] = ..., executor_class: _Optional[_Union[ExecutorClass, str]] = ..., secret_scope: _Optional[_Union[SecretScope, _Mapping]] = ...) -> None: ...

class SecretScope(_message.Message):
    __slots__ = ("none", "allowlist")
    NONE_FIELD_NUMBER: _ClassVar[int]
    ALLOWLIST_FIELD_NUMBER: _ClassVar[int]
    none: SecretScopeNone
    allowlist: SecretRefAllowlist
    def __init__(self, none: _Optional[_Union[SecretScopeNone, _Mapping]] = ..., allowlist: _Optional[_Union[SecretRefAllowlist, _Mapping]] = ...) -> None: ...

class SecretScopeNone(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class SecretRefAllowlist(_message.Message):
    __slots__ = ("names",)
    NAMES_FIELD_NUMBER: _ClassVar[int]
    names: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, names: _Optional[_Iterable[str]] = ...) -> None: ...

class SubmitMoteRequest(_message.Message):
    __slots__ = ("mote", "warrant", "accept_at_least_once", "react_seed")
    MOTE_FIELD_NUMBER: _ClassVar[int]
    WARRANT_FIELD_NUMBER: _ClassVar[int]
    ACCEPT_AT_LEAST_ONCE_FIELD_NUMBER: _ClassVar[int]
    REACT_SEED_FIELD_NUMBER: _ClassVar[int]
    mote: Mote
    warrant: WarrantSpec
    accept_at_least_once: bool
    react_seed: bool
    def __init__(self, mote: _Optional[_Union[Mote, _Mapping]] = ..., warrant: _Optional[_Union[WarrantSpec, _Mapping]] = ..., accept_at_least_once: bool = ..., react_seed: bool = ...) -> None: ...

class SubmitMoteResponse(_message.Message):
    __slots__ = ("mote_id", "status", "detail", "instance_id", "refusal_code")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    REFUSAL_CODE_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    status: SubmitStatus
    detail: str
    instance_id: bytes
    refusal_code: str
    def __init__(self, mote_id: _Optional[bytes] = ..., status: _Optional[_Union[SubmitStatus, str]] = ..., detail: _Optional[str] = ..., instance_id: _Optional[bytes] = ..., refusal_code: _Optional[str] = ...) -> None: ...

class ReportCommitRequest(_message.Message):
    __slots__ = ("mote_id", "idempotency_key", "result_ref", "warrant_ref", "mote_def_hash", "nd_class", "parents", "worker_id")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    RESULT_REF_FIELD_NUMBER: _ClassVar[int]
    WARRANT_REF_FIELD_NUMBER: _ClassVar[int]
    MOTE_DEF_HASH_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    PARENTS_FIELD_NUMBER: _ClassVar[int]
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    idempotency_key: bytes
    result_ref: bytes
    warrant_ref: bytes
    mote_def_hash: bytes
    nd_class: NdClass
    parents: _containers.RepeatedCompositeFieldContainer[ParentRef]
    worker_id: int
    def __init__(self, mote_id: _Optional[bytes] = ..., idempotency_key: _Optional[bytes] = ..., result_ref: _Optional[bytes] = ..., warrant_ref: _Optional[bytes] = ..., mote_def_hash: _Optional[bytes] = ..., nd_class: _Optional[_Union[NdClass, str]] = ..., parents: _Optional[_Iterable[_Union[ParentRef, _Mapping]]] = ..., worker_id: _Optional[int] = ...) -> None: ...

class ReportCommitResponse(_message.Message):
    __slots__ = ("committed_seq", "outcome", "detail")
    COMMITTED_SEQ_FIELD_NUMBER: _ClassVar[int]
    OUTCOME_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    committed_seq: int
    outcome: CommitOutcome
    detail: str
    def __init__(self, committed_seq: _Optional[int] = ..., outcome: _Optional[_Union[CommitOutcome, str]] = ..., detail: _Optional[str] = ...) -> None: ...

class ReportEffectStagedRequest(_message.Message):
    __slots__ = ("mote_id", "idempotency_key", "worker_id")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    idempotency_key: bytes
    worker_id: int
    def __init__(self, mote_id: _Optional[bytes] = ..., idempotency_key: _Optional[bytes] = ..., worker_id: _Optional[int] = ...) -> None: ...

class ReportEffectStagedResponse(_message.Message):
    __slots__ = ("staged_seq", "ack")
    STAGED_SEQ_FIELD_NUMBER: _ClassVar[int]
    ACK_FIELD_NUMBER: _ClassVar[int]
    staged_seq: int
    ack: bool
    def __init__(self, staged_seq: _Optional[int] = ..., ack: bool = ...) -> None: ...

class ReportFailureRequest(_message.Message):
    __slots__ = ("mote_id", "idempotency_key", "reason_class", "worker_id")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    REASON_CLASS_FIELD_NUMBER: _ClassVar[int]
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    idempotency_key: bytes
    reason_class: FailureReason
    worker_id: int
    def __init__(self, mote_id: _Optional[bytes] = ..., idempotency_key: _Optional[bytes] = ..., reason_class: _Optional[_Union[FailureReason, str]] = ..., worker_id: _Optional[int] = ...) -> None: ...

class ReportFailureResponse(_message.Message):
    __slots__ = ("failed_seq", "ack")
    FAILED_SEQ_FIELD_NUMBER: _ClassVar[int]
    ACK_FIELD_NUMBER: _ClassVar[int]
    failed_seq: int
    ack: bool
    def __init__(self, failed_seq: _Optional[int] = ..., ack: bool = ...) -> None: ...

class HeartbeatRequest(_message.Message):
    __slots__ = ("worker_id", "timestamp_ms", "in_flight")
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    TIMESTAMP_MS_FIELD_NUMBER: _ClassVar[int]
    IN_FLIGHT_FIELD_NUMBER: _ClassVar[int]
    worker_id: int
    timestamp_ms: int
    in_flight: int
    def __init__(self, worker_id: _Optional[int] = ..., timestamp_ms: _Optional[int] = ..., in_flight: _Optional[int] = ...) -> None: ...

class HeartbeatResponse(_message.Message):
    __slots__ = ("ack",)
    ACK_FIELD_NUMBER: _ClassVar[int]
    ack: bool
    def __init__(self, ack: bool = ...) -> None: ...

class RegisterWorkerRequest(_message.Message):
    __slots__ = ("executor_class", "endpoint")
    EXECUTOR_CLASS_FIELD_NUMBER: _ClassVar[int]
    ENDPOINT_FIELD_NUMBER: _ClassVar[int]
    executor_class: ExecutorClass
    endpoint: str
    def __init__(self, executor_class: _Optional[_Union[ExecutorClass, str]] = ..., endpoint: _Optional[str] = ...) -> None: ...

class RegisterWorkerResponse(_message.Message):
    __slots__ = ("worker_id",)
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    worker_id: int
    def __init__(self, worker_id: _Optional[int] = ...) -> None: ...

class LeaseWorkRequest(_message.Message):
    __slots__ = ("worker_id", "executor_class", "max_motes")
    WORKER_ID_FIELD_NUMBER: _ClassVar[int]
    EXECUTOR_CLASS_FIELD_NUMBER: _ClassVar[int]
    MAX_MOTES_FIELD_NUMBER: _ClassVar[int]
    worker_id: int
    executor_class: ExecutorClass
    max_motes: int
    def __init__(self, worker_id: _Optional[int] = ..., executor_class: _Optional[_Union[ExecutorClass, str]] = ..., max_motes: _Optional[int] = ...) -> None: ...

class ParentResult(_message.Message):
    __slots__ = ("parent_mote_id", "result_ref")
    PARENT_MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    RESULT_REF_FIELD_NUMBER: _ClassVar[int]
    parent_mote_id: bytes
    result_ref: bytes
    def __init__(self, parent_mote_id: _Optional[bytes] = ..., result_ref: _Optional[bytes] = ...) -> None: ...

class ToolArgs(_message.Message):
    __slots__ = ("args_bytes", "net_scope", "fs_scope")
    ARGS_BYTES_FIELD_NUMBER: _ClassVar[int]
    NET_SCOPE_FIELD_NUMBER: _ClassVar[int]
    FS_SCOPE_FIELD_NUMBER: _ClassVar[int]
    args_bytes: bytes
    net_scope: NetScope
    fs_scope: FsScope
    def __init__(self, args_bytes: _Optional[bytes] = ..., net_scope: _Optional[_Union[NetScope, _Mapping]] = ..., fs_scope: _Optional[_Union[FsScope, _Mapping]] = ...) -> None: ...

class WorkItem(_message.Message):
    __slots__ = ("mote", "warrant", "parent_results", "tool_args", "context_items", "image_ref")
    MOTE_FIELD_NUMBER: _ClassVar[int]
    WARRANT_FIELD_NUMBER: _ClassVar[int]
    PARENT_RESULTS_FIELD_NUMBER: _ClassVar[int]
    TOOL_ARGS_FIELD_NUMBER: _ClassVar[int]
    CONTEXT_ITEMS_FIELD_NUMBER: _ClassVar[int]
    IMAGE_REF_FIELD_NUMBER: _ClassVar[int]
    mote: Mote
    warrant: WarrantSpec
    parent_results: _containers.RepeatedCompositeFieldContainer[ParentResult]
    tool_args: ToolArgs
    context_items: bytes
    image_ref: bytes
    def __init__(self, mote: _Optional[_Union[Mote, _Mapping]] = ..., warrant: _Optional[_Union[WarrantSpec, _Mapping]] = ..., parent_results: _Optional[_Iterable[_Union[ParentResult, _Mapping]]] = ..., tool_args: _Optional[_Union[ToolArgs, _Mapping]] = ..., context_items: _Optional[bytes] = ..., image_ref: _Optional[bytes] = ...) -> None: ...

class LeaseWorkResponse(_message.Message):
    __slots__ = ("items", "instance_id")
    ITEMS_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    items: _containers.RepeatedCompositeFieldContainer[WorkItem]
    instance_id: bytes
    def __init__(self, items: _Optional[_Iterable[_Union[WorkItem, _Mapping]]] = ..., instance_id: _Optional[bytes] = ...) -> None: ...

class ReadEntriesRequest(_message.Message):
    __slots__ = ("since_seq", "max")
    SINCE_SEQ_FIELD_NUMBER: _ClassVar[int]
    MAX_FIELD_NUMBER: _ClassVar[int]
    since_seq: int
    max: int
    def __init__(self, since_seq: _Optional[int] = ..., max: _Optional[int] = ...) -> None: ...

class CommittedEntry(_message.Message):
    __slots__ = ("mote_id", "idempotency_key", "seq", "nd_class", "result_ref", "parents", "warrant_ref", "mote_def_hash")
    MOTE_ID_FIELD_NUMBER: _ClassVar[int]
    IDEMPOTENCY_KEY_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    ND_CLASS_FIELD_NUMBER: _ClassVar[int]
    RESULT_REF_FIELD_NUMBER: _ClassVar[int]
    PARENTS_FIELD_NUMBER: _ClassVar[int]
    WARRANT_REF_FIELD_NUMBER: _ClassVar[int]
    MOTE_DEF_HASH_FIELD_NUMBER: _ClassVar[int]
    mote_id: bytes
    idempotency_key: bytes
    seq: int
    nd_class: NdClass
    result_ref: bytes
    parents: _containers.RepeatedCompositeFieldContainer[ParentRef]
    warrant_ref: bytes
    mote_def_hash: bytes
    def __init__(self, mote_id: _Optional[bytes] = ..., idempotency_key: _Optional[bytes] = ..., seq: _Optional[int] = ..., nd_class: _Optional[_Union[NdClass, str]] = ..., result_ref: _Optional[bytes] = ..., parents: _Optional[_Iterable[_Union[ParentRef, _Mapping]]] = ..., warrant_ref: _Optional[bytes] = ..., mote_def_hash: _Optional[bytes] = ...) -> None: ...

class JournalEntry(_message.Message):
    __slots__ = ("seq", "committed")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    COMMITTED_FIELD_NUMBER: _ClassVar[int]
    seq: int
    committed: CommittedEntry
    def __init__(self, seq: _Optional[int] = ..., committed: _Optional[_Union[CommittedEntry, _Mapping]] = ...) -> None: ...

class ReadEntriesResponse(_message.Message):
    __slots__ = ("entries", "next_seq")
    ENTRIES_FIELD_NUMBER: _ClassVar[int]
    NEXT_SEQ_FIELD_NUMBER: _ClassVar[int]
    entries: _containers.RepeatedCompositeFieldContainer[JournalEntry]
    next_seq: int
    def __init__(self, entries: _Optional[_Iterable[_Union[JournalEntry, _Mapping]]] = ..., next_seq: _Optional[int] = ...) -> None: ...

class RegisterRunRequest(_message.Message):
    __slots__ = ("recipe_fingerprint",)
    RECIPE_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    recipe_fingerprint: bytes
    def __init__(self, recipe_fingerprint: _Optional[bytes] = ...) -> None: ...

class RegisterRunResponse(_message.Message):
    __slots__ = ("instance_id",)
    INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    instance_id: bytes
    def __init__(self, instance_id: _Optional[bytes] = ...) -> None: ...
