/**
 * Model-free gRPC-web stub harness. A Playwright `page.route` layer that SYNTHESIZES
 * gRPC-web `application/grpc-web+proto` responses so a model-driven console flow can be
 * driven deterministically with NO served model — the missing piece the console e2e never
 * had (every other spec spawns a real, model-free `kx serve`, so the propose→diff→approve
 * gate and the NL ProposeWorkflow path were only ever asserted as "wired", never fired).
 *
 * The design keeps the net HONEST: only the model-INFERENCE RPCs are stubbed. Branch
 * mutations (`AdvanceBranch`) and reads (`GetBranch` / `GetBranchContent`) are left to
 * hit the REAL gateway, so `approve` genuinely advances the real manifest and the test
 * can read the mutation back from the server — a behavioural net, not a render check.
 *
 * COUPLING (read-only, intentional): the message schemas come from the SDK's public
 * `proto` re-export (`@kortecx/sdk/node`); the protobuf runtime (`create` / `toBinary` /
 * `fromBinary`) is reached through the SDK's own dependency tree because `ui` does not
 * depend on `@bufbuild/protobuf` directly. If a future SDK refactor drops the `proto`
 * re-export or moves the dep, update these two imports.
 */

import { REF_LEN, asBytes, proto } from "@kortecx/sdk/node";
import type { Page, Route } from "@playwright/test";
// @bufbuild/protobuf is the SDK's dependency (not hoisted to ui); reach its ESM entry.
import {
  create,
  fromBinary,
  toBinary,
} from "../../../bindings/typescript/node_modules/@bufbuild/protobuf/dist/esm/index.js";
import { SPA_ORIGIN } from "./serve";

const MESSAGE_FLAG = 0x00;
const TRAILER_FLAG = 0x80;
const GATEWAY = "/kortecx.v1.KxGateway";

/** One length-prefixed gRPC-web envelope frame: [flag][uint32 BE length][bytes]. */
function frame(flag: number, bytes: Uint8Array): Uint8Array {
  const out = new Uint8Array(5 + bytes.length);
  out[0] = flag;
  new DataView(out.buffer).setUint32(1, bytes.length, false); // gRPC-web length is big-endian
  out.set(bytes, 5);
  return out;
}

/**
 * A successful unary gRPC-web body: one message frame + a `grpc-status: 0` trailer frame
 * (connect-web reads the status from the trailer when there is no grpc-status HTTP header).
 */
function unaryBody(messageBytes: Uint8Array): Buffer {
  const message = frame(MESSAGE_FLAG, messageBytes);
  const trailer = frame(TRAILER_FLAG, new TextEncoder().encode("grpc-status: 0\r\n"));
  return Buffer.concat([message, trailer]);
}

/** Fulfill a POST route with a canned unary response (+ the CORS header the cross-origin SPA needs). */
async function fulfillUnary(route: Route, messageBytes: Uint8Array): Promise<void> {
  await route.fulfill({
    status: 200,
    headers: {
      "content-type": "application/grpc-web+proto",
      "access-control-allow-origin": SPA_ORIGIN,
      "access-control-expose-headers": "grpc-status,grpc-message,grpc-status-details-bin",
    },
    body: unaryBody(messageBytes),
  });
}

/** The protobuf request payload, with the 5-byte gRPC-web envelope header stripped. */
function reqBytes(route: Route): Uint8Array {
  const buf = route.request().postDataBuffer();
  if (buf === null) {
    throw new Error("gRPC-web request had no POST body");
  }
  return new Uint8Array(buf).subarray(5);
}

function bytesEq(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) {
    return false;
  }
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) {
      return false;
    }
  }
  return true;
}

/** Register a POST-only route on a gateway method; the CORS preflight (OPTIONS) passes through. */
async function onMethod(
  page: Page,
  method: string,
  handler: (route: Route) => Promise<void>,
): Promise<void> {
  await page.route(
    (u) => u.pathname === `${GATEWAY}/${method}`,
    (route) => (route.request().method() === "POST" ? handler(route) : route.continue()),
  );
}

export interface ReactEditStub {
  /** A 64-hex ref of a blob ALREADY in the gateway's content store — the proposed file body. */
  resultRef: string;
  /** The exact bytes `GetContent` returns (the proposed file body). */
  proposedBytes: Uint8Array;
}

/**
 * Stub the model-inference leg of `editBranchPropose` — `Invoke("kx/recipes/react-edit")`
 * → `GetProjection` (the terminal mote commits to `resultRef`) → `GetContent` (returns the
 * proposed body). The embedded AppChat's own `Invoke("kx/recipes/chat")`, and every other
 * RPC (`GetBranch`, `GetBranchContent`, `AdvanceBranch`), fall through to the real gateway,
 * so `approve` advances the REAL manifest to a REAL ref.
 */
export async function stubReactEdit(page: Page, stub: ReactEditStub): Promise<void> {
  const instanceId = new Uint8Array(16).fill(0x11);
  const terminalMoteId = new Uint8Array(32).fill(0x22);
  const refBytes = asBytes(stub.resultRef, REF_LEN);

  await onMethod(page, "Invoke", async (route) => {
    const req = fromBinary(proto.InvokeRequestSchema, reqBytes(route));
    if (req.handle !== "kx/recipes/react-edit") {
      await route.continue(); // the embedded AppChat's chat Invoke reaches the real gateway
      return;
    }
    const resp = create(proto.InvokeResponseSchema, {
      instanceId,
      terminalMoteId,
      recipeFingerprint: new Uint8Array(32),
      reactChainSalt: new Uint8Array(0), // empty ⇒ the non-react terminal-mote wait path
    });
    await fulfillUnary(route, toBinary(proto.InvokeResponseSchema, resp));
  });

  await onMethod(page, "GetProjection", async (route) => {
    const req = fromBinary(proto.GetProjectionRequestSchema, reqBytes(route));
    if (!bytesEq(req.instanceId, instanceId)) {
      await route.continue(); // some other run's projection — leave it to the real gateway
      return;
    }
    const view = create(proto.ProjectionViewSchema, {
      instanceId,
      recipeFingerprint: new Uint8Array(32),
      currentSeq: 1n,
      motes: [
        create(proto.MoteSnapshotSchema, {
          moteId: terminalMoteId,
          state: proto.MoteSnapshotState.COMMITTED,
          resultRef: refBytes,
        }),
      ],
    });
    await fulfillUnary(route, toBinary(proto.ProjectionViewSchema, view));
  });

  await onMethod(page, "GetContent", async (route) => {
    const req = fromBinary(proto.GetContentRequestSchema, reqBytes(route));
    if (!bytesEq(req.contentRef, refBytes)) {
      await route.continue(); // some other ref — leave it to the real gateway
      return;
    }
    const blob = create(proto.ContentBlobSchema, { payload: stub.proposedBytes });
    await fulfillUnary(route, toBinary(proto.ContentBlobSchema, blob));
  });
}

export interface ProposedStepInit {
  role: string;
  intent: string;
  kind?: string;
  modelId?: string;
}

/**
 * Stub `ProposeWorkflow` with a canned multi-step plan. A single unary RPC — reused by the
 * NL-authoring specs to drive `useProposeWorkflow` past the model-less gateway (which, being
 * model-free, would otherwise honestly reject the proposal).
 */
export async function stubProposeWorkflow(
  page: Page,
  plan: { steps: ProposedStepInit[]; edges: { parent: number; child: number }[] },
): Promise<void> {
  await onMethod(page, "ProposeWorkflow", async (route) => {
    const resp = create(proto.ProposeWorkflowResponseSchema, {
      result: {
        case: "plan",
        value: create(proto.ProposedPlanSchema, {
          steps: plan.steps.map((s) =>
            create(proto.ProposedStepSchema, {
              role: s.role,
              intent: s.intent,
              kind: s.kind ?? "plain",
              modelId: s.modelId ?? "",
            }),
          ),
          edges: plan.edges.map((e) => create(proto.ProposedEdgeSchema, e)),
        }),
      },
    });
    await fulfillUnary(route, toBinary(proto.ProposeWorkflowResponseSchema, resp));
  });
}
