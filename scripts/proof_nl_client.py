#!/usr/bin/env python3
"""The Rule-41 proof's gRPC client for the NL authoring surface.

Drives `DescribeControlSurface` / `ProposeControlAction` over the REAL wire with the
REAL generated stubs — not a hand-rolled shim. A stub speaks the protocol you wrote;
the generated client speaks the protocol the server actually serves, and the gap
between those two is where a wire defect lives.

## `approve` is the assertion, not a convenience

`approve` does NOT rebuild a request from the preview's rendered fields. It takes the
`ControlPreview` message the server returned, pulls the request out of its `oneof`,
and sends THAT MESSAGE to the named RPC. That is the property the whole design exists
for — approving forwards the bytes that were displayed — so the proof exercises it
rather than trusting it.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bindings/python/src"))

import grpc  # noqa: E402
from google.protobuf import json_format  # noqa: E402

from kortecx.v1 import gateway_pb2 as g  # noqa: E402
from kortecx.v1 import gateway_pb2_grpc as gg  # noqa: E402


def stub(endpoint: str):
    target = endpoint.removeprefix("http://").removeprefix("https://")
    return gg.KxGatewayStub(grpc.insecure_channel(target))


def cmd_describe(args) -> int:
    s = stub(args.endpoint)
    resp = s.DescribeControlSurface(
        g.DescribeControlSurfaceRequest(domain=args.domain, authoring_only=False)
    )
    # `always_print_fields_with_no_presence` is load-bearing here, not cosmetic.
    # Without it proto3 omits every FALSE bool, so `mutates: false` and
    # `authoring: false` vanish from the JSON — and an assertion that a proposal
    # RPC does not mutate would read a missing key rather than a false value. The
    # first run of this proof died on exactly that (`KeyError: 'authoring'`), which
    # is the good version of the failure: the alternative is a `.get(k, False)`
    # that cannot tell "the server said false" from "the server said nothing".
    print(
        json.dumps(
            json_format.MessageToDict(
                resp,
                preserving_proto_field_name=True,
                always_print_fields_with_no_presence=True,
            )
        )
    )
    return 0


def cmd_propose(args) -> int:
    s = stub(args.endpoint)
    resp = s.ProposeControlAction(
        g.ProposeControlActionRequest(goal=args.goal, domain=args.domain)
    )
    which = resp.WhichOneof("result")
    if which == "rejected":
        print(json.dumps({"kind": "rejected", "reason": resp.rejected.reason}))
        return 0
    if which != "preview":
        print(json.dumps({"kind": "empty", "reason": "the server returned no result"}))
        return 0

    p = resp.preview
    # Carry the preview's WIRE BYTES through, so `approve` forwards exactly what was
    # displayed rather than a reconstruction of it. Anything that re-derived the
    # request here would be testing the reconstruction.
    print(
        json.dumps(
            {
                "kind": "preview",
                "rpc": p.rpc,
                "summary": p.summary,
                "request_field": p.WhichOneof("request"),
                "preview_b64": _b64(p.SerializeToString()),
                "rendered": json_format.MessageToDict(
                    p,
                    preserving_proto_field_name=True,
                    always_print_fields_with_no_presence=True,
                ),
            }
        )
    )
    return 0


def cmd_approve(args) -> int:
    doc = json.loads(Path(args.preview).read_text())
    if doc.get("kind") != "preview":
        print(json.dumps({"kind": "error", "detail": "not a preview document"}))
        return 1

    preview = g.ControlPreview()
    preview.ParseFromString(_unb64(doc["preview_b64"]))
    field = preview.WhichOneof("request")
    if field is None:
        print(json.dumps({"kind": "error", "detail": "the preview carries no request"}))
        return 1

    # The request message the server put in the preview, forwarded verbatim.
    req = getattr(preview, field)
    s = stub(args.endpoint)

    # rpc name -> the stub method. Explicit rather than getattr(s, preview.rpc) so an
    # unexpected rpc name is a REFUSAL here, not a reflective call into whatever
    # happens to share the name.
    dispatch = {
        "SaveWorkflow": s.SaveWorkflow,
        "RegisterTool": s.RegisterTool,
        "RegisterMcpServer": s.RegisterMcpServer,
        "RegisterTrigger": s.RegisterTrigger,
        "PutPolicyRole": s.PutPolicyRole,
        "AssignPolicyRole": s.AssignPolicyRole,
    }
    call = dispatch.get(preview.rpc)
    if call is None:
        # Secrets and scripts ride REDUCED preview arms: the preview is deliberately
        # not the real request (no value, no argv/env), so approving one is an
        # operator action with information the model never had. Saying so is the
        # honest outcome, not silently constructing the missing fields.
        print(
            json.dumps(
                {
                    "kind": "not-forwardable",
                    "detail": (
                        f"{preview.rpc} rides a REDUCED preview arm — the operator "
                        "supplies what the proposal structurally cannot carry"
                    ),
                }
            )
        )
        return 0

    resp = call(req)
    print(
        json.dumps(
            {
                "kind": "applied",
                "rpc": preview.rpc,
                "detail": f"{preview.rpc} accepted the previewed bytes verbatim",
                "response": json_format.MessageToDict(
                    resp, preserving_proto_field_name=True
                ),
            }
        )
    )
    return 0


def _b64(b: bytes) -> str:
    import base64

    return base64.b64encode(b).decode()


def _unb64(s: str) -> bytes:
    import base64

    return base64.b64decode(s)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    d = sub.add_parser("describe")
    d.add_argument("--endpoint", required=True)
    d.add_argument("--domain", default="")
    d.set_defaults(fn=cmd_describe)

    p = sub.add_parser("propose")
    p.add_argument("--endpoint", required=True)
    p.add_argument("--goal", required=True)
    p.add_argument("--domain", default="")
    p.set_defaults(fn=cmd_propose)

    a = sub.add_parser("approve")
    a.add_argument("--endpoint", required=True)
    a.add_argument("--preview", required=True)
    a.set_defaults(fn=cmd_approve)

    args = ap.parse_args()
    try:
        return args.fn(args)
    except grpc.RpcError as e:  # a transport/status failure is the proof's answer
        print(json.dumps({"kind": "rpc-error", "code": str(e.code()), "reason": e.details()}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
