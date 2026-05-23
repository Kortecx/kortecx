// Per `03-ffi-and-inference.md` §1, this is the **single unsafe boundary** of the
// runtime. `#![forbid(unsafe_code)]` is intentionally OMITTED here.

#![allow(missing_docs)]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::all)]

//! # kx-llamacpp-sys — raw FFI to llama.cpp's C API
//!
//! The single unsafe-heavy crate. Bindings are generated at build time by `bindgen`
//! against the vendored llama.cpp submodule (pinned to a specific upstream tag via
//! `git submodule`).
//!
//! **Nothing but `kx-llamacpp` (P1.7) may import this crate.** The rule is enforced
//! by convention; future workspace lints will catch violations.
//!
//! ## Pinned upstream version
//!
//! [`PINNED_LLAMACPP_TAG`] records the git tag the submodule is pinned to. The
//! build script invokes CMake against that source tree.

/// The upstream llama.cpp git tag this crate's build script compiles against.
///
/// Updates: bump the submodule (`git -C kx-llamacpp-sys/llama.cpp checkout <new_tag>;
/// git add kx-llamacpp-sys/llama.cpp`) and update this string in lock-step.
pub const PINNED_LLAMACPP_TAG: &str = "b9000";

// The generated bindings live at OUT_DIR/bindings.rs. They're verbose (1000s of
// lines for the full llama.h surface) so they're not pre-committed — they're
// regenerated on every build by `build.rs`.
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
