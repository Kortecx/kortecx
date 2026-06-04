//! Build script: compile the gRPC schema with `tonic-build`, using the pinned
//! **vendored** `protoc` so neither CI nor local dev needs a system protobuf
//! compiler (SN-7: ships linux-x86_64 + macos-aarch64). Generation is
//! deterministic — pinned protoc + fixed `.proto` produce byte-identical Rust
//! across the two clean release builds of the byte-determinism gate (I1.c).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Point tonic-build / prost-build at the vendored protoc binary instead of
    // requiring one on PATH. `set_var` is safe on edition 2021.
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/kortecx/v1/coordinator.proto",
                "proto/kortecx/v1/gateway.proto",
            ],
            &["proto"],
        )?;

    println!("cargo:rerun-if-changed=proto/kortecx/v1/coordinator.proto");
    println!("cargo:rerun-if-changed=proto/kortecx/v1/gateway.proto");
    Ok(())
}
