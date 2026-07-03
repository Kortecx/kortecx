"""The fluent App builder — author a durable, reusable App (a ``kortecx.app/v1``
envelope) over an existing Flow/Chain blueprint (POC-4).

```python
import kortecx as kx

app = (kx.app("research-assistant")
       .blueprint(kx.flow().agent("Research the topic", tools=["mcp-echo/echo"]))
       .rule("no-pii", body="Never reveal personal data.")
       .steer(max_turns=8, max_tool_calls=6)
       .describe("A grounded research agent"))

app.save()              # persist to the catalog (uploads any pending bodies first)
app.run({"topic": "kortecx"})   # compile the blueprint + run it (exactly-once)
```

An App WRAPS a blueprint (the byte-stable ``to_blueprint()`` output) with a minimal
prompt/rule/skill/memory reference rail, a 4-axis steering config, and per-step
replay intent. It carries NO authority — ``run`` re-compiles the blueprint and the
server re-resolves every warrant from the caller's own grants (SN-8). The envelope
serializes byte-identically to the Rust ``kx-app`` + the TS SDK (the golden corpus).
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Dict, List, Mapping, Optional, Union

from .apps import APP_SCHEMA, SaveAppResult, Skill, pretty_json
from .chains import Chain, ChainError

if TYPE_CHECKING:
    from .flow import Flow
    from .run import Result, Run


def _is_hex_ref(s: str) -> bool:
    return len(s) == 64 and all(c in "0123456789abcdef" for c in s)


#: G1: the curated Gmail provider defaults (the bundled ``kx-connector-gmail`` sidecar).
#: Mirrors the CLI ``kx connections add --provider gmail`` + the UI provider catalog.
GMAIL_CONNECTOR_COMMAND = "kx-connector-gmail"
GMAIL_CREDENTIAL_REF = "KX_GMAIL_CREDENTIAL"

#: RC-SW2: the curated Discord provider defaults (the bundled ``kx-connector-discord``
#: sidecar, #277). Mirrors ``with_gmail`` — register with
#: ``kx connections add --command kx-connector-discord --credential-ref KX_DISCORD_CREDENTIAL``.
DISCORD_CONNECTOR_COMMAND = "kx-connector-discord"
DISCORD_CREDENTIAL_REF = "KX_DISCORD_CREDENTIAL"


class App:
    """A fluent App builder. Each method returns ``self``; terminate with
    :meth:`to_envelope` / :meth:`export` / :meth:`save` / :meth:`run`."""

    def __init__(self, name: str, *, version: str = "1", seed: int = 0) -> None:
        self._name = name
        self._version = version
        self._seed = seed
        self._blueprint: Optional[Dict[str, Any]] = None
        self._description = ""
        self._tags: List[str] = []
        self._input_schema: Optional[Dict[str, Any]] = None
        # by-ref reference rails (each entry already content-addressed).
        self._rails: Dict[str, List[Dict[str, Any]]] = {
            "context": [],
            "tools": [],
            "connections": [],
            "datasets": [],
            "prompts": [],
            "rules": [],
            "skills": [],
            "memory": [],
        }
        # pending text bodies to upload at save(): (rail, name, body, skill_or_None).
        self._pending: List[tuple] = []
        self._model_route = ""
        self._free_params: Dict[str, str] = {}
        self._requested_grants: Dict[str, str] = {}
        self._max_turns: Optional[int] = None
        self._max_tool_calls: Optional[int] = None
        # G2: secret NAMES to expose at run (guards.secret_scope). Populated by
        # with_connection(scope_secret=True); the server narrows the run warrant's
        # SecretScope::AllowList to these (bounded by the referenced connections).
        self._secret_scope: List[str] = []
        self._branch_handle = ""
        self._replay: Dict[str, str] = {}
        # RC-SW2: imperative pre-run registrations carried from a Flow via
        # :meth:`Flow.as_app` (with_mcp connectors / with_memory facts). Run() executes
        # them before RunApp so a `flow().with_mcp(...).as_app(...).run()` chain still
        # registers its connector. Never part of the envelope (off the golden digest).
        self._flow_mcp: List[Dict[str, Any]] = []
        self._flow_memory: List[Dict[str, Any]] = []

    # -- composition --

    def blueprint(self, source: "Union[Flow, Chain]") -> "App":
        """Capture the run topology from a :class:`~kortecx.flow.Flow` or
        :class:`~kortecx.chains.Chain` via its byte-stable ``to_blueprint()``."""
        self._blueprint = source.to_blueprint()
        return self

    def _add_artifact(self, rail: str, name: str, ref: Optional[str], body: Optional[str]) -> "App":
        if ref is not None:
            if not _is_hex_ref(ref):
                raise ChainError(f"{rail} ref must be 64-char lowercase hex, got {ref!r}")
            self._rails[rail].append({"name": name, "content_ref": ref})
        elif body is not None:
            self._pending.append((rail, name, body, None))
        else:
            raise ChainError(f"{rail}({name!r}) needs either ref= or body=")
        return self

    def prompt(self, name: str, *, ref: Optional[str] = None, body: Optional[str] = None) -> "App":
        """Add a prompt artifact — a named text body in the content store."""
        return self._add_artifact("prompts", name, ref, body)

    def rule(self, name: str, *, ref: Optional[str] = None, body: Optional[str] = None) -> "App":
        """Add a rule artifact (a governance/behavior note)."""
        return self._add_artifact("rules", name, ref, body)

    def memory(self, name: str, *, ref: Optional[str] = None, body: Optional[str] = None) -> "App":
        """Add a memory artifact (a named context note)."""
        return self._add_artifact("memory", name, ref, body)

    def skill(self, skill: Skill) -> "App":
        """Add a skill — a named (instructions + tool wish SET) bundle ≈ an Agent."""
        if skill.instructions_ref:
            if not _is_hex_ref(skill.instructions_ref):
                raise ChainError("skill instructions_ref must be 64-char lowercase hex")
            self._rails["skills"].append(
                {
                    "name": skill.name,
                    "instructions_ref": skill.instructions_ref,
                    **({"tools": dict(skill.tools)} if skill.tools else {}),
                }
            )
        elif skill.instructions:
            self._pending.append(("skills", skill.name, skill.instructions, skill))
        else:
            raise ChainError(f"skill {skill.name!r} needs instructions or instructions_ref")
        return self

    def context(self, name: str, ref: str, *, media_type: str = "") -> "App":
        """Reference a context item by content ref (carries ``media_type``)."""
        if not _is_hex_ref(ref):
            raise ChainError("context ref must be 64-char lowercase hex")
        entry: Dict[str, Any] = {"name": name, "content_ref": ref}
        if media_type:
            entry["media_type"] = media_type
        self._rails["context"].append(entry)
        return self

    def use_tool(self, tool_id: str, tool_version: str = "1") -> "App":
        """Reference a registered tool (id + version only — never a grant)."""
        self._rails["tools"].append({"tool_id": tool_id, "tool_version": tool_version})
        return self

    def dataset(self, dataset_ref: str, *, cas_refs: Optional[List[str]] = None) -> "App":
        """Ground the App on a dataset (declarative RAG-on-App). At run, ``RunApp`` grants
        the entry step the read-only ``retrieve`` tool and steers it to search
        ``dataset_ref`` live in the loop — the App self-grounds instead of needing a
        hand-authored blueprint. INGEST the corpus first with ``kx datasets ingest
        <dataset_ref> …`` (the "reference-existing" model; a named dataset absent from the
        server fails closed at run). ``cas_refs`` (64-hex content refs the dataset spans)
        are recorded for a future self-contained ingest; today grounding uses the
        pre-ingested named dataset."""
        entry: Dict[str, Any] = {"dataset_ref": dataset_ref}
        if cas_refs:
            for r in cas_refs:
                if not _is_hex_ref(r):
                    raise ChainError(f"dataset cas_ref must be 64-char lowercase hex, got {r!r}")
            entry["cas_refs"] = list(cas_refs)
        self._rails["datasets"].append(entry)
        return self

    def rag(self, dataset_ref: str, *, cas_refs: Optional[List[str]] = None) -> "App":
        """Alias for :meth:`dataset` — ground the App on a dataset (RAG-on-App)."""
        return self.dataset(dataset_ref, cas_refs=cas_refs)

    def with_connection(
        self, descriptor: str, credential_ref: str = "", *, scope_secret: bool = True
    ) -> "App":
        """G2: declare a by-reference connection the App uses. ``descriptor`` is the MCP
        endpoint (a stdio command or an ``http(s)`` URL, no userinfo); ``credential_ref``
        is the bare secret NAME the runtime resolves at DIAL time (never the value). By
        default the credential is also added to ``guards.secret_scope`` so the run
        warrant permits dialing it (``RunApp`` narrows ``SecretScope::AllowList`` to
        these); pass ``scope_secret=False`` for a credential-less connection. The
        pointer is a bare name, so a shared App resolves each operator's OWN
        credentials — register the connection with ``kx connections add`` first."""
        self._rails["connections"].append(
            {"descriptor": descriptor, "credential_ref": credential_ref}
        )
        if scope_secret and credential_ref and credential_ref not in self._secret_scope:
            self._secret_scope.append(credential_ref)
        return self

    def with_gmail(self) -> "App":
        """G1: declare the bundled Gmail connector (the curated provider default) —
        equivalent to ``with_connection("kx-connector-gmail", "KX_GMAIL_CREDENTIAL")``.
        Register it on the runtime with ``kx connections add --provider gmail``."""
        return self.with_connection(GMAIL_CONNECTOR_COMMAND, GMAIL_CREDENTIAL_REF)

    def with_discord(self) -> "App":
        """RC-SW2: declare the bundled Discord connector (the curated provider default) —
        equivalent to ``with_connection("kx-connector-discord", "KX_DISCORD_CREDENTIAL")``.
        Register it on the runtime with ``kx connections add --command kx-connector-discord
        --credential-ref KX_DISCORD_CREDENTIAL`` (the sidecar reads a bot token by name)."""
        return self.with_connection(DISCORD_CONNECTOR_COMMAND, DISCORD_CREDENTIAL_REF)

    def secrets(self, names: "Union[str, List[str]]") -> "App":
        """Add secret NAMES to ``guards.secret_scope`` — the run warrant's
        ``SecretScope::AllowList`` narrows to these so a granted connector may be dialed
        inside the agentic loop (G2/#285). The VALUE never travels (D81). A scope name is
        **bounded server-side by the referenced connections** — pair it with the matching
        :meth:`with_connection` / :meth:`with_gmail` / :meth:`with_discord`, or the entry
        is inert (fails closed). Usually implicit via ``with_connection(scope_secret=True)``;
        use this to declare scope explicitly."""
        for name in [names] if isinstance(names, str) else names:
            if name and name not in self._secret_scope:
                self._secret_scope.append(name)
        return self

    def _carry_flow_side_channels(
        self, mcp: "List[Dict[str, Any]]", memory: "List[Dict[str, Any]]"
    ) -> None:
        """Carry a promoting Flow's imperative side-channels (``with_mcp`` connectors +
        ``with_memory`` facts) so :meth:`run` registers them before ``RunApp``. Set by
        :meth:`Flow.as_app`; never part of the envelope."""
        self._flow_mcp = list(mcp)
        self._flow_memory = list(memory)

    # -- steering (4 axes; the server RE-RESOLVES each at bind) --

    def steer(
        self,
        *,
        model: str = "",
        max_turns: Optional[int] = None,
        max_tool_calls: Optional[int] = None,
        requested_grants: Optional[Mapping[str, str]] = None,
        free_params: Optional[Mapping[str, str]] = None,
    ) -> "App":
        """Set steering knobs (a WISH the server re-resolves at bind — never authority)."""
        if model:
            self._model_route = model
        if max_turns is not None:
            self._max_turns = max_turns
        if max_tool_calls is not None:
            self._max_tool_calls = max_tool_calls
        if requested_grants:
            self._requested_grants.update(requested_grants)
        if free_params:
            self._free_params.update({k: str(v) for k, v in free_params.items()})
        return self

    def tags(self, *tags: str) -> "App":
        """Add catalog tags."""
        self._tags.extend(tags)
        return self

    def describe(self, text: str) -> "App":
        """Set the advisory description."""
        self._description = text
        return self

    def branch(self, handle: str) -> "App":
        """Set the (optional) per-App project branch handle (reserved; never created here)."""
        self._branch_handle = handle
        return self

    # -- terminals --

    def _references_dict(self) -> Dict[str, Any]:
        return {rail: items for rail, items in self._rails.items() if items}

    def _steering_dict(self) -> Dict[str, Any]:
        steer: Dict[str, Any] = {}
        model: Dict[str, Any] = {}
        if self._model_route:
            model["model_route"] = self._model_route
        if self._free_params:
            model["free_params"] = dict(self._free_params)
        if model:
            steer["model"] = model
        if self._requested_grants:
            steer["tools"] = {"requested_grants": dict(self._requested_grants)}
        guards: Dict[str, Any] = {}
        if self._max_turns is not None:
            guards["max_turns"] = self._max_turns
        if self._max_tool_calls is not None:
            guards["max_tool_calls"] = self._max_tool_calls
        if self._secret_scope:
            # Dedup, preserve declaration order (the server sorts into a BTreeSet).
            guards["secret_scope"] = list(dict.fromkeys(self._secret_scope))
        if guards:
            steer["guards"] = guards
        return steer

    def to_envelope(self) -> Dict[str, Any]:
        """Assemble the ``kortecx.app/v1`` envelope dict (omit-empty, the canonical
        byte-shape). Requires the blueprint and NO pending body uploads — use
        :meth:`save` (which uploads pending bodies first) or pass artifacts by ``ref``."""
        if self._blueprint is None:
            raise ChainError("app needs a blueprint — call .blueprint(flow()/chain(...))")
        if self._pending:
            names = ", ".join(f"{rail}:{name}" for rail, name, _b, _s in self._pending)
            raise ChainError(
                f"to_envelope() cannot resolve pending body uploads ({names}); "
                "use .save(client=...) or pass artifacts by ref="
            )
        env: Dict[str, Any] = {
            "schema": APP_SCHEMA,
            "name": self._name,
            "version": self._version,
            "blueprint": self._blueprint,
        }
        if self._description:
            env["description"] = self._description
        if self._tags:
            env["tags"] = list(self._tags)
        if self._input_schema is not None:
            env["input_schema"] = self._input_schema
        refs = self._references_dict()
        if refs:
            env["references"] = refs
        steer = self._steering_dict()
        if steer:
            env["steering_config"] = steer
        if self._replay:
            env["replay"] = {"per_step": dict(self._replay)}
        if self._branch_handle:
            env["branch_handle"] = self._branch_handle
        return env

    def export(self, path) -> None:
        """Write the pretty envelope JSON to ``path`` (the round-trip artifact)."""
        with open(path, "w", encoding="utf-8") as f:
            f.write(pretty_json(self.to_envelope()))

    def _resolve_pending(self, client) -> None:
        """Upload pending text bodies to the content store, turning them into refs."""
        for rail, name, body, skill in self._pending:
            ref = client.put_content(body.encode("utf-8"), media_type="text/plain").content_ref
            if rail == "skills":
                entry: Dict[str, Any] = {"name": name, "instructions_ref": ref}
                if skill is not None and skill.tools:
                    entry["tools"] = dict(skill.tools)
                self._rails["skills"].append(entry)
            else:
                self._rails[rail].append({"name": name, "content_ref": ref})
        self._pending = []

    def save(self, *, handle: Optional[str] = None, client=None) -> SaveAppResult:
        """Upload any pending bodies, then ``SaveApp`` the canonical envelope. The
        handle defaults to ``apps/local/<sanitized-name>``."""
        from .defaults import default_client

        kx = client if client is not None else default_client()
        self._resolve_pending(kx)
        return kx.save_app(self.to_envelope(), handle=handle)

    def run(
        self,
        args: Optional[Mapping[str, object]] = None,
        *,
        wait: bool = True,
        timeout: float = 120.0,
        client=None,
    ) -> "Union[Run, Result]":
        """Save this App and run it via ``RunApp`` (exactly-once). ``args`` (the App's
        input schema) fold server-side into the entry step's prompt.

        RC-SW2 fix: this now routes through ``SaveApp`` + ``RunApp`` instead of a local
        ``submit_workflow`` recompile — so the App's ``references.connections`` +
        ``guards.secret_scope`` reach the server and a credentialed connector (Gmail /
        Discord) actually fires inside the agentic loop (the G2/#285 path). Saving is
        expected: an ``App`` is an explicitly-named durable object (``kx.app(name)`` /
        ``flow().as_app(name)``); the save is idempotent (content-addressed envelope +
        handle upsert). The server re-resolves every warrant from the caller's grants
        (SN-8)."""
        from .defaults import default_client

        kx = client if client is not None else default_client()
        self._resolve_pending(kx)
        # Imperative side-channels carried from a promoting Flow (Flow.as_app).
        for spec in self._flow_mcp:
            kx.register_mcp_server(**spec)
        for fact in self._flow_memory:
            kx.store_memory(**fact)
        saved = kx.save_app(self.to_envelope())
        str_args = {str(k): str(v) for k, v in dict(args).items()} if args else None
        return kx.run_app(saved.handle, args=str_args, wait=wait, timeout=timeout)


def app(name: str, *, version: str = "1", seed: int = 0) -> App:
    """Start an App: ``kx.app("my-app").blueprint(kx.flow()...).save()``. The
    authoring container that WRAPS a blueprint into a durable, reusable App."""
    return App(name, version=version, seed=seed)


def minimal_app_envelope(name: str, goal: str, *, model: str = "") -> Dict[str, Any]:
    """POC-5a: author a MINIMAL App envelope (a single agentic step over ``goal``)
    for the "New App" one-shot — save it, then ``client.scaffold_app(handle)``
    scaffolds the project tree into its branch. The envelope carries NO authority
    (the server re-resolves warrants at run); the blueprint is a valid single-step
    DAG."""
    from .flow import flow

    builder = app(name).describe(goal).blueprint(flow().agent(goal))
    if model:
        builder.steer(model=model)
    return builder.to_envelope()
