/* kx-llamacpp-sys — bindgen aggregation header.
 *
 * Single entry point for `bindgen` so ONE pass emits ONE `bindings.rs` covering
 * both the llama.cpp C API and the mtmd (multi-modal) C API, with no duplicate
 * `llama_*`/`ggml_*` type definitions.
 *
 * Parsed as C (build.rs does NOT pass `-x c++`): every `#ifdef __cplusplus`
 * block is skipped, so the C++ STL includes (`<string>`, `<vector>`, …) and the
 * C++ smart-pointer deleter section in mtmd.h never reach the parser — only the
 * `extern "C"` surface is bound. `mtmd-helper.h` transitively includes `mtmd.h`,
 * `llama.h`, and `ggml.h`; the explicit `llama.h` include below is harmless
 * (header guards prevent double definition) and keeps the intent legible.
 *
 * Committed (not generated) so `bindgen` output stays byte-reproducible across
 * builds (the `check-reproducible` / I1.c gate).
 */
#include "llama.h"
#include "mtmd-helper.h"
