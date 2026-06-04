//! D78 — assemble the Mote's upstream context + tool menu into the model input.
//!
//! This is the wiring of the (previously unwired) `kx_context_assembler::assemble`
//! into the real-model dispatch path. Before a model Mote dispatches, the seam:
//!
//! 1. reads the orchestrator's published [`kx_projection::Snapshot`] (D78 seam —
//!    [`kx_runtime::SnapshotSink`]);
//! 2. calls `assemble` to resolve the Mote's Data-edge parent `result_ref` bytes
//!    ‖ the resolved tool-description bytes per `tool_grant`, bounded by a real
//!    byte budget derived from the warrant (replacing the previous unbounded
//!    `usize::MAX`);
//! 3. composes those bytes with the Mote's instruction into the model input.
//!
//! **Leaf Motes are byte-unchanged.** A model Mote with no Data parents and no
//! tool grants assembles an *empty* context, so the model input is exactly
//! `chatml(instruction)` — identical to the pre-D78 path (the harness's existing
//! A–J rows). Only a Mote with real upstream context / tools takes the new path.
//!
//! **`assemble` stays pure** (D78): it reads committed bytes through the
//! snapshot, content store, and tool registry; nothing here adds nondeterminism,
//! and the assembled context is **model input only** — it never enters Mote
//! identity or the journal (D64). The instruction itself remains identity-bearing
//! via `config_subset["prompt"]` (see [`crate::prompt`]).

use kx_content::{sniff_image_format, ContentStore};
use kx_context_assembler::{assemble, AssembledContext, AssembledItem, AssemblyError};
use kx_inference::{InferenceInput, MEDIA_MARKER};
use kx_mote::Mote;
use kx_runtime::SnapshotSink;
use kx_tool_registry::ToolRegistry;
use kx_warrant::WarrantSpec;

use crate::prompt;

/// Derive the assemble byte budget from the warrant's model route.
///
/// The assembler bounds the assembled closure to this many bytes, failing closed
/// with [`AssemblyError::OverflowDecisionRequired`] on overflow (never a panic).
/// We use the assemble doc's `~4 bytes/token` heuristic over the warrant's
/// `max_input_tokens` — a real, warrant-scoped cap rather than the unbounded
/// `usize::MAX` every caller passed before this wiring. Saturating, so a large
/// token ceiling can never wrap.
#[must_use]
pub(crate) fn window_bytes_from_warrant(warrant: &WarrantSpec) -> usize {
    (warrant.model_route.max_input_tokens as usize).saturating_mul(4)
}

/// Render an [`AssembledContext`] into a single prompt fragment.
///
/// **This mirrors the canonical convention in `kx_inference`'s (private)
/// `serialize_context`** (`crates/kx-inference/src/dispatcher.rs`): each item is
/// `LABEL:\n<utf8-lossy bytes>\n\n`, concatenated in the context's deterministic
/// item order. It is duplicated here (crate-private, fenced by the
/// `assemble_wiring` integration test) because that function is not public and
/// `kx-inference` is a frozen-trio crate we must not modify. Wiring the full
/// `Dispatcher::dispatch_mote` (so this single convention is reused, not
/// duplicated) is the M5.2 follow-up — see the M5.1 forward-enhancement note.
///
/// **Only `label` + `bytes` are rendered — never `source_ref`** (the D78
/// "no hash reaches the window" invariant; `bytes` is content, never a hash).
///
/// Renders only the supplied (text) items. Image items are routed out as
/// `content_ref`s (see [`model_input`]); their raw bytes never enter the text
/// prompt — a media marker stands in for each.
fn render_context(items: &[&AssembledItem]) -> String {
    let mut out = String::new();
    for item in items {
        out.push_str(&item.label);
        out.push_str(":\n");
        out.push_str(&String::from_utf8_lossy(&item.bytes));
        out.push_str("\n\n");
    }
    out
}

/// Build the [`InferenceInput`] for a model Mote (D78): its instruction plus any
/// assembled upstream context + tool menu, ChatML-wrapped.
///
/// `instruction` is the Mote's identity-bearing prompt (from
/// `config_subset["prompt"]`). When the snapshot is available and the Mote has
/// Data parents / tool grants, their resolved bytes are assembled and prepended
/// into the user turn; otherwise the input is exactly `chatml(instruction)`
/// (byte-identical to the pre-D78 leaf path).
///
/// # Errors
///
/// Propagates [`AssemblyError`] verbatim — notably
/// [`AssemblyError::OverflowDecisionRequired`] when the assembled closure
/// exceeds the warrant-derived window (a typed shaper-decision seam, never a
/// panic). The caller maps it into its own seam error.
pub(crate) fn model_input<S: ContentStore>(
    mote: &Mote,
    warrant: &WarrantSpec,
    instruction: &str,
    sink: &SnapshotSink,
    store: &S,
    registry: &dyn ToolRegistry,
) -> Result<InferenceInput, AssemblyError> {
    // No published snapshot (e.g. a direct unit call without the orchestrator)
    // ⇒ no upstream context to assemble ⇒ the leaf path.
    let ctx = match sink.latest() {
        Some(snapshot) => assemble(
            mote,
            warrant,
            &snapshot,
            store,
            registry,
            window_bytes_from_warrant(warrant),
        )?,
        None => AssembledContext::default(),
    };

    // Partition the assembled items by modality. Image-sniffed parents flow to
    // the projector as `content_ref`s (PR-2); everything else is text the model
    // reads in the window. A text-only closure takes the exact pre-PR-2 path.
    let mut text_items: Vec<&AssembledItem> = Vec::with_capacity(ctx.items.len());
    let mut image_refs: Vec<kx_content::ContentRef> = Vec::new();
    for item in &ctx.items {
        if sniff_image_format(&item.bytes).is_some() {
            image_refs.push(item.source_ref);
        } else {
            text_items.push(item);
        }
    }

    if image_refs.is_empty() {
        // Text-only path — BYTE-IDENTICAL to the pre-multimodal behavior.
        let user_turn = if text_items.is_empty() {
            instruction.to_string()
        } else {
            // D78: the resolved tool-menu + parent bytes reach the model, ahead
            // of the instruction, inside the ChatML user turn.
            format!("{}{instruction}", render_context(&text_items))
        };
        return Ok(InferenceInput::text(prompt::chatml(&user_turn)));
    }

    // Multi-modal path: one media marker per image at the head of the user turn
    // (the projector splices each image in marker-order), then any text context,
    // then the instruction — ChatML-wrapped. The image BYTES never enter the
    // text; they ride as `content_ref`s the backend fetches + decodes.
    let markers = MEDIA_MARKER.repeat(image_refs.len());
    let user_turn = format!("{markers}{}{instruction}", render_context(&text_items));
    Ok(InferenceInput::Multimodal {
        text: prompt::chatml(&user_turn),
        content_refs: image_refs.into_iter().collect(),
    })
}
