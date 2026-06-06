//! Environment-driven inference tuning knobs (CPU + Apple-Metal).
//!
//! These are read ONCE inside [`crate::ModelParams::new`] and
//! [`crate::ContextParams::new`] — the exact constructors the (frozen)
//! `kx-inference` dispatch path already calls (`Model::load` →
//! `ModelParams::new`; `ContextParams::new().with_n_ctx(..)`). Reading the env
//! there means GPU offload, flash-attention, KV-cache quantization, and thread
//! tuning flow into the runtime path with **zero edits to the frozen trio**
//! (`kx-executor`/`kx-scheduler`/`kx-inference`).
//!
//! Every parser is **total + panic-free**: an unset OR unrecognized value
//! yields `None`, and the constructor keeps llama.cpp's upstream default. So
//! `env-unset` is **byte-identical to the pre-change behavior** on non-Apple
//! platforms — preserving the determinism smoke tests and the canonical
//! product digest (which is driven by the deterministic executor, not llama).
//!
//! CUDA stays cloud-only (D28): the platform default offload is all-layers on
//! Apple (Metal is compiled in) and CPU (`0`) everywhere else. No `cuda`
//! feature is introduced.
//!
//! The pure `parse_*` helpers are split from the env reads so they can be
//! unit-tested without touching process-global env (which would race under the
//! parallel test runner).

use crate::context::{FlashAttn, KvCacheType};

/// `KX_N_GPU_LAYERS` env var name.
pub(crate) const ENV_N_GPU_LAYERS: &str = "KX_N_GPU_LAYERS";
/// `KX_FLASH_ATTN` env var name.
pub(crate) const ENV_FLASH_ATTN: &str = "KX_FLASH_ATTN";
/// `KX_KV_TYPE` env var name.
pub(crate) const ENV_KV_TYPE: &str = "KX_KV_TYPE";
/// `KX_N_THREADS` env var name.
pub(crate) const ENV_N_THREADS: &str = "KX_N_THREADS";

/// Parse `KX_N_GPU_LAYERS`: `all`/`-1` (case-insensitive) ⇒ all layers (`-1`);
/// a parseable `i32` ⇒ that count; anything else ⇒ `None`.
pub(crate) fn parse_n_gpu_layers(raw: &str) -> Option<i32> {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("all") {
        return Some(-1);
    }
    v.parse::<i32>().ok()
}

/// Parse `KX_FLASH_ATTN`: `auto` / `on`/`off` (+ common synonyms) ⇒ the enum;
/// anything else ⇒ `None`.
pub(crate) fn parse_flash_attn(raw: &str) -> Option<FlashAttn> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(FlashAttn::Auto),
        "on" | "1" | "true" | "enabled" | "enable" => Some(FlashAttn::Enabled),
        "off" | "0" | "false" | "disabled" | "disable" => Some(FlashAttn::Disabled),
        _ => None,
    }
}

/// Parse `KX_KV_TYPE`: `f16` / `q8_0` (alias `q8`) ⇒ the enum; else `None`.
pub(crate) fn parse_kv_cache_type(raw: &str) -> Option<KvCacheType> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "f16" => Some(KvCacheType::F16),
        "q8_0" | "q8" => Some(KvCacheType::Q8_0),
        _ => None,
    }
}

/// Parse `KX_N_THREADS`: a positive `i32` ⇒ that count; else `None` (keep
/// llama.cpp's auto default).
pub(crate) fn parse_n_threads(raw: &str) -> Option<i32> {
    match raw.trim().parse::<i32>() {
        Ok(n) if n > 0 => Some(n),
        _ => None,
    }
}

/// The platform default GPU offload when `KX_N_GPU_LAYERS` is unset: all layers
/// on Apple (Metal compiled in), CPU (`0`) elsewhere (CUDA is cloud-only, D28).
pub(crate) fn default_n_gpu_layers() -> i32 {
    if cfg!(target_os = "macos") {
        -1
    } else {
        0
    }
}

/// `KX_N_GPU_LAYERS` resolved (env override else `None`).
pub(crate) fn n_gpu_layers() -> Option<i32> {
    std::env::var(ENV_N_GPU_LAYERS)
        .ok()
        .as_deref()
        .and_then(parse_n_gpu_layers)
}

/// `KX_FLASH_ATTN` resolved.
pub(crate) fn flash_attn() -> Option<FlashAttn> {
    std::env::var(ENV_FLASH_ATTN)
        .ok()
        .as_deref()
        .and_then(parse_flash_attn)
}

/// `KX_KV_TYPE` resolved.
pub(crate) fn kv_cache_type() -> Option<KvCacheType> {
    std::env::var(ENV_KV_TYPE)
        .ok()
        .as_deref()
        .and_then(parse_kv_cache_type)
}

/// `KX_N_THREADS` resolved.
pub(crate) fn n_threads() -> Option<i32> {
    std::env::var(ENV_N_THREADS)
        .ok()
        .as_deref()
        .and_then(parse_n_threads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n_gpu_layers_all_and_neg_one_mean_all() {
        assert_eq!(parse_n_gpu_layers("all"), Some(-1));
        assert_eq!(parse_n_gpu_layers("ALL"), Some(-1));
        assert_eq!(parse_n_gpu_layers(" -1 "), Some(-1));
    }

    #[test]
    fn n_gpu_layers_integer() {
        assert_eq!(parse_n_gpu_layers("0"), Some(0));
        assert_eq!(parse_n_gpu_layers("32"), Some(32));
    }

    #[test]
    fn n_gpu_layers_garbage_is_none() {
        assert_eq!(parse_n_gpu_layers(""), None);
        assert_eq!(parse_n_gpu_layers("lots"), None);
        assert_eq!(parse_n_gpu_layers("3.5"), None);
    }

    #[test]
    fn flash_attn_synonyms() {
        assert_eq!(parse_flash_attn("auto"), Some(FlashAttn::Auto));
        assert_eq!(parse_flash_attn("ON"), Some(FlashAttn::Enabled));
        assert_eq!(parse_flash_attn("true"), Some(FlashAttn::Enabled));
        assert_eq!(parse_flash_attn("off"), Some(FlashAttn::Disabled));
        assert_eq!(parse_flash_attn("0"), Some(FlashAttn::Disabled));
        assert_eq!(parse_flash_attn("nonsense"), None);
        assert_eq!(parse_flash_attn(""), None);
    }

    #[test]
    fn kv_type_values() {
        assert_eq!(parse_kv_cache_type("f16"), Some(KvCacheType::F16));
        assert_eq!(parse_kv_cache_type("F16"), Some(KvCacheType::F16));
        assert_eq!(parse_kv_cache_type("q8_0"), Some(KvCacheType::Q8_0));
        assert_eq!(parse_kv_cache_type("q8"), Some(KvCacheType::Q8_0));
        assert_eq!(parse_kv_cache_type("q4_k"), None);
        assert_eq!(parse_kv_cache_type(""), None);
    }

    #[test]
    fn n_threads_positive_only() {
        assert_eq!(parse_n_threads("8"), Some(8));
        assert_eq!(parse_n_threads("0"), None);
        assert_eq!(parse_n_threads("-4"), None);
        assert_eq!(parse_n_threads("auto"), None);
    }

    #[test]
    fn default_offload_is_platform_correct() {
        // The compiled default differs by host: all layers on Apple, CPU else.
        let expect = if cfg!(target_os = "macos") { -1 } else { 0 };
        assert_eq!(default_n_gpu_layers(), expect);
    }
}
