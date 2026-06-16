//! `kx-llamacpp-sys/build.rs` — builds llama.cpp and generates Rust bindings.
//!
//! Per `03-ffi-and-inference.md` §1 + D28:
//! - llama.cpp is the OSS in-process inference backend.
//! - CUDA is disabled (GPU-batched serving is cloud-side per D28).
//! - llama.cpp's native platform defaults handle Metal on Apple Silicon, CPU elsewhere.
//! - Static linking so downstream binaries are self-contained.
//!
//! What this script does:
//! 1. Builds llama.cpp's static libraries via CMake (the `cmake` crate drives it).
//! 2. Generates Rust bindings via `bindgen` against llama.h, with an allowlist
//!    that limits the surface to `llama_*` symbols.
//! 3. Emits the cargo directives so the downstream rlib links the produced library.

// TODO(workspace.lints cleanup): the [workspace.lints] policy in the root
// Cargo.toml turns on `unwrap_used`, `expect_used`, and the `pedantic` group
// for every compilation unit — including build scripts. This script reads
// build-time env vars (CARGO_MANIFEST_DIR, OUT_DIR) where panic-on-missing is
// the appropriate behavior (a missing env var means the cargo invocation is
// malformed; build scripts SHOULD panic loudly). A future cleanup PR will
// migrate these to `expect("CARGO_MANIFEST_DIR set by cargo")` style with
// the workspace policy migrating to `deny`. For now, allow at the
// build-script level so the workspace baseline doesn't break the build.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_statements,
    clippy::doc_markdown,
    // The pin-echo + dual CMake/bindgen setup makes `main` a linear script that
    // reads top-to-bottom; splitting it for a line count would hurt legibility.
    clippy::too_many_lines
)]

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let llama_cpp_dir = manifest_dir.join("llama.cpp");

    // Invalidate the build if the submodule contents change. The submodule pointer
    // is the load-bearing pin; rebuilds happen when llama.cpp is updated.
    println!("cargo:rerun-if-changed=llama.cpp/include/llama.h");
    println!("cargo:rerun-if-changed=llama.cpp/tools/mtmd/mtmd.h");
    println!("cargo:rerun-if-changed=llama.cpp/tools/mtmd/mtmd-helper.h");
    println!("cargo:rerun-if-changed=llama.cpp/CMakeLists.txt");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");

    // ------------------------------------------------------------------
    // Pin echo — surface the llama.cpp submodule SHA at build time so CI
    // logs and local developer terminals show drift detectably. The pin
    // documentation + upgrade procedure lives at `PIN.md` next to this
    // file; any unaudited submodule advance changes this echo and a
    // grep over CI logs surfaces it.
    //
    // Failure to read the submodule's HEAD is a soft-warn — the build
    // continues (don't block builds on a missing `.git`), but the warning
    // makes the drift visible.
    // ------------------------------------------------------------------
    if let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(&llama_cpp_dir)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
    {
        if output.status.success() {
            let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // Capture the short version-tag-derived name for human readability.
            let describe = std::process::Command::new("git")
                .arg("-C")
                .arg(&llama_cpp_dir)
                .arg("describe")
                .arg("--tags")
                .arg("--always")
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "unknown".to_string());
            println!(
                "cargo:warning=kx-llamacpp-sys: linking against llama.cpp pin {sha} ({describe}); \
                 see crates/kx-llamacpp-sys/PIN.md for the audit procedure"
            );
        } else {
            println!(
                "cargo:warning=kx-llamacpp-sys: could not read llama.cpp submodule HEAD \
                 (git rev-parse failed); pin drift will not be surfaced this build"
            );
        }
    } else {
        println!(
            "cargo:warning=kx-llamacpp-sys: git not on PATH; llama.cpp pin SHA \
             not echoed (this is fine in vendored builds but means pin drift is invisible)"
        );
    }

    // ------------------------------------------------------------------
    // 1. Build llama.cpp via CMake.
    // ------------------------------------------------------------------
    let mut config = cmake::Config::new(&llama_cpp_dir);
    config
        // Static libraries — no .so/.dylib runtime dependencies.
        .define("BUILD_SHARED_LIBS", "OFF")
        // No tests, no examples, no server (they bloat the build and pull deps).
        .define("LLAMA_BUILD_TESTS", "OFF")
        .define("LLAMA_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_BUILD_SERVER", "OFF")
        // The unified `llama` app binary (added upstream 2026-06) links
        // `llama-server-impl`, which `LLAMA_BUILD_SERVER=OFF` does not build —
        // so the default-ON app target fails to link. We only need the static
        // libs (ggml/llama/mtmd), so disable it. NOTE: `LLAMA_BUILD_TOOLS` MUST
        // stay ON — `tools/mtmd` (libmtmd.a, the multi-modal projector) lives
        // under `tools/`.
        .define("LLAMA_BUILD_APP", "OFF")
        // CUDA is cloud-side per D28. Disable for OSS portability.
        .define("GGML_CUDA", "OFF")
        // BLAS optional; disable to keep the build dep-light.
        .define("GGML_BLAS", "OFF")
        // OpenMP off: avoids the libgomp link-time requirement on Linux
        // (rust-lld doesn't auto-link it). ggml-cpu falls back to its own
        // thread-pool. Slight CPU perf cost; acceptable for OSS single-compute
        // scope per D28. Cloud-side serving uses vLLM / SGLang (P5.1 / P5.1.5)
        // which handle their own batching + threading.
        .define("GGML_OPENMP", "OFF")
        // Embed Metal shader library directly into the static archive on
        // Apple targets so downstream binaries don't need to ship a separate
        // `default.metallib` next to the executable. No effect on non-Apple.
        .define("GGML_METAL_EMBED_LIBRARY", "ON")
        // Position-independent code for downstream static linking.
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        // Use Release optimization for the C++ build to keep runtime perf.
        .profile("Release");

    // aarch64-LINUX baseline: GCC won't inline the `__fp16` NEON intrinsics
    // ggml-cpu's `vec.cpp`/`ops.cpp` use (`vfmaq_f16`/`vaddq_f16`) without an arch
    // that enables FP16 arithmetic — the default build fails with "inlining failed
    // in call to 'always_inline' 'vfmaq_f16' … target specific option mismatch"
    // (hit when building the Linux/arm64 inference image, e.g. Docker on Apple
    // Silicon). ggml's default `GGML_NATIVE=ON` path runs `-mcpu=native` detection
    // which on GCC-12 yields `+fp16fml` but NOT the base `+fp16` arithmetic the
    // intrinsics need; and `GGML_CPU_ARM_ARCH` is only honored when `GGML_NATIVE`
    // is OFF (ggml `ggml-cpu/CMakeLists.txt`). So set BOTH: disable native
    // detection and pin `armv8.2-a+fp16` (the modern-arm64 baseline — Apple
    // Silicon, Graviton2+, Ampere). SCOPED to aarch64-linux so the aarch64-APPLE
    // (Apple clang enables fp16 itself) + x86_64 builds are byte-unchanged: macOS
    // native + amd64 CI never take this branch.
    let cmake_target = env::var("TARGET").unwrap_or_default();
    if cmake_target.contains("aarch64") && cmake_target.contains("linux") {
        config
            .define("GGML_NATIVE", "OFF")
            .define("GGML_CPU_ARM_ARCH", "armv8.2-a+fp16");
    }

    let dst = config.build();

    // ------------------------------------------------------------------
    // 2. Tell cargo about the build output + libraries.
    // ------------------------------------------------------------------
    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // llama.cpp's static archives produced by `BUILD_SHARED_LIBS=OFF`.
    // Linking order matters for static libs — dependencies after dependents.
    // `mtmd` (multi-modal: vision/audio projector + clip + stb/miniaudio decode,
    // with `mtmd-helper` compiled in) depends on `llama` + `ggml`, so it must
    // precede them. It is already produced by the CMake build (the `cmake` crate
    // builds llama.cpp as top-level source → LLAMA_STANDALONE=ON → LLAMA_BUILD_TOOLS
    // ON → tools/mtmd builds libmtmd.a); PR-2 only adds the link + the bindings.
    // `libmtmd.a` is `llama-common`-free (its CMakeLists FATAL_ERROR-guards that),
    // so no extra archive is needed.
    println!("cargo:rustc-link-lib=static=mtmd");
    println!("cargo:rustc-link-lib=static=llama");
    println!("cargo:rustc-link-lib=static=ggml");
    println!("cargo:rustc-link-lib=static=ggml-base");
    println!("cargo:rustc-link-lib=static=ggml-cpu");

    // Platform-conditional libs: link the C++ standard library; on macOS link the
    // Foundation / Accelerate / Metal frameworks that llama.cpp's default build
    // expects on Apple Silicon. On Linux, link libstdc++.
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("apple") {
        // Metal backend (only built on Apple targets by cmake).
        println!("cargo:rustc-link-lib=static=ggml-metal");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Accelerate");
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=MetalKit");
        println!("cargo:rustc-link-lib=framework=MetalPerformanceShaders");
        println!("cargo:rustc-link-lib=c++");
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }

    // ------------------------------------------------------------------
    // 3. Generate Rust bindings via bindgen.
    // ------------------------------------------------------------------
    // `wrapper.h` (committed, crate root) aggregates llama.h + mtmd-helper.h so a
    // single bindgen pass emits one bindings.rs with no duplicate llama_*/ggml_*
    // types. Parsed as C (no `-x c++`): the `#ifdef __cplusplus` blocks (mtmd.h's
    // C++ STL includes + smart-pointer deleters) are skipped, leaving only the
    // `extern "C"` surface.
    let wrapper_h = manifest_dir.join("wrapper.h");
    let include_dir = llama_cpp_dir.join("include");
    let ggml_include = llama_cpp_dir.join("ggml").join("include");
    let mtmd_dir = llama_cpp_dir.join("tools").join("mtmd");

    let bindings = bindgen::Builder::default()
        .header(wrapper_h.to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.display()))
        .clang_arg(format!("-I{}", ggml_include.display()))
        // mtmd.h / mtmd-helper.h live under tools/mtmd.
        .clang_arg(format!("-I{}", mtmd_dir.display()))
        // Allowlist: llama_* (+ the closely-related ggml types llama.h
        // transitively references) and mtmd_* (the multi-modal C API:
        // mtmd_init_from_file, mtmd_tokenize, mtmd_helper_eval_chunks,
        // mtmd_helper_bitmap_init_from_buf, …). Keeps the surface manageable.
        .allowlist_function("llama_.*")
        .allowlist_function("mtmd_.*")
        .allowlist_type("llama_.*")
        .allowlist_type("mtmd_.*")
        .allowlist_var("LLAMA_.*")
        .allowlist_var("MTMD_.*")
        // Use core types so the bindings work in no_std contexts too.
        .use_core()
        .ctypes_prefix("::core::ffi")
        // Generate Rust enums for known C enums; rustified for usability.
        .rustified_enum("llama_.*")
        .rustified_enum("mtmd_.*")
        // Block clang from looking at system headers it doesn't need.
        .layout_tests(false)
        .derive_default(true)
        .derive_debug(true)
        // Cargo invalidation hook for header changes.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Failed to generate llama.cpp bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Failed to write bindings.rs to OUT_DIR");
}
