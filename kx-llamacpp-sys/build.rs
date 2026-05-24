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

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let llama_cpp_dir = manifest_dir.join("llama.cpp");

    // Invalidate the build if the submodule contents change. The submodule pointer
    // is the load-bearing pin; rebuilds happen when llama.cpp is updated.
    println!("cargo:rerun-if-changed=llama.cpp/include/llama.h");
    println!("cargo:rerun-if-changed=llama.cpp/CMakeLists.txt");
    println!("cargo:rerun-if-changed=build.rs");

    // ------------------------------------------------------------------
    // 1. Build llama.cpp via CMake.
    // ------------------------------------------------------------------
    let dst = cmake::Config::new(&llama_cpp_dir)
        // Static libraries — no .so/.dylib runtime dependencies.
        .define("BUILD_SHARED_LIBS", "OFF")
        // No tests, no examples, no server (they bloat the build and pull deps).
        .define("LLAMA_BUILD_TESTS", "OFF")
        .define("LLAMA_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_BUILD_SERVER", "OFF")
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
        .profile("Release")
        .build();

    // ------------------------------------------------------------------
    // 2. Tell cargo about the build output + libraries.
    // ------------------------------------------------------------------
    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // llama.cpp's static archives produced by `BUILD_SHARED_LIBS=OFF`.
    // Linking order matters for static libs — dependencies after dependents.
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
    let llama_h = llama_cpp_dir.join("include").join("llama.h");
    let include_dir = llama_cpp_dir.join("include");
    let ggml_include = llama_cpp_dir.join("ggml").join("include");

    let bindings = bindgen::Builder::default()
        .header(llama_h.to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.display()))
        .clang_arg(format!("-I{}", ggml_include.display()))
        // Allowlist: only llama_* (and the closely-related ggml types llama_h
        // transitively references). Keeps the binding surface manageable.
        .allowlist_function("llama_.*")
        .allowlist_type("llama_.*")
        .allowlist_var("LLAMA_.*")
        // Use core types so the bindings work in no_std contexts too.
        .use_core()
        .ctypes_prefix("::core::ffi")
        // Generate Rust enums for known C enums; rustified for usability.
        .rustified_enum("llama_.*")
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
