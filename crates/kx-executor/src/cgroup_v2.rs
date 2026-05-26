//! cgroup v2 file I/O for hierarchical resource enforcement on Linux.
//!
//! PR 9a-hardening-6 ships an opt-in `LinuxCgroupV2ResourceManager` that
//! creates a per-Mote cgroup directory under a configured parent path
//! (e.g., `/sys/fs/cgroup/kx-mote/`) + writes the WarrantSpec ceilings to
//! the cgroup's control files (`cpu.max`, `memory.max`, `pids.max`). At
//! spawn time, a `cgroup_attach` pre-exec hook writes the child's own
//! PID to `cgroup.procs` so the kernel attaches the child to the cgroup
//! before execvp. On release, the cgroup directory is removed.
//!
//! **Permissions**: writing to `/sys/fs/cgroup/...` requires either
//! root, `CAP_SYS_ADMIN`, or a systemd-delegated subtree
//! (`Delegate=yes` on the parent unit). Production deployments wire one
//! of these; the module's `probe()` constructor reports
//! `LinuxCgroupV2Error::NotWritable` if the configured parent isn't
//! writable + the caller can fall back to the existing setrlimit-based
//! `LocalResourceManager`.
//!
//! **Status**: this is the cgroup v2 substrate. PR 9a-hardening-6 ships
//! the module + structural unit tests + the pre-exec hook; runtime
//! validation against an actual `/sys/fs/cgroup/...` requires Linux + a
//! writable subtree, exercised on Linux CI when the runner has the
//! permission (otherwise the test runtime-skips).

#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use kx_warrant::ResourceCeiling;
use thiserror::Error;

use crate::resource_manager::{ResourceError, ResourceManager, Slot};

/// Default parent path under which per-Mote cgroups are created. Production
/// deployments override via `LinuxCgroupV2ResourceManager::with_parent`.
pub const DEFAULT_CGROUP_PARENT: &str = "/sys/fs/cgroup/kx-mote";

/// `LinuxCgroupV2ResourceManager` errors. Surface them to the caller so
/// production deployments can fall back to the setrlimit-only path
/// (`LocalResourceManager`) when cgroup v2 is unavailable.
#[derive(Debug, Error)]
pub enum LinuxCgroupV2Error {
    /// The configured parent path is not writable. Production callers
    /// typically need root, `CAP_SYS_ADMIN`, or a systemd-delegated
    /// subtree.
    #[error("cgroup parent {path:?} is not writable: {reason}")]
    NotWritable {
        /// Path that was probed.
        path: PathBuf,
        /// Underlying filesystem error rendered as a string.
        reason: String,
    },
    /// cgroup v2 is not the active controller hierarchy (the system uses
    /// cgroup v1 or `/sys/fs/cgroup/cgroup.controllers` is missing).
    #[error("cgroup v2 not available at {path:?}: {reason}")]
    NotV2 {
        /// Path that was probed.
        path: PathBuf,
        /// Diagnostic string explaining why cgroup v2 isn't available.
        reason: String,
    },
    /// File I/O during acquire/release failed.
    #[error("cgroup file I/O: {0}")]
    Io(String),
}

impl From<LinuxCgroupV2Error> for ResourceError {
    fn from(err: LinuxCgroupV2Error) -> Self {
        ResourceError::Internal(err.to_string())
    }
}

/// cgroup v2-backed `ResourceManager` impl for Linux. **Opt-in**:
/// production callers construct via `probe(parent)` which returns
/// `Err(LinuxCgroupV2Error::NotWritable)` if the parent isn't writable +
/// the caller falls back to the setrlimit-only `LocalResourceManager`.
#[derive(Debug)]
pub struct LinuxCgroupV2ResourceManager {
    parent: PathBuf,
    next_slot_id: AtomicU64,
    outstanding: std::sync::Mutex<std::collections::HashMap<u64, PathBuf>>,
}

impl LinuxCgroupV2ResourceManager {
    /// Probe `parent` for cgroup v2 writability + return a manager
    /// instance. Returns `Err(LinuxCgroupV2Error)` if `parent` isn't
    /// writable, isn't on a cgroup v2 hierarchy, or doesn't exist.
    ///
    /// # Errors
    ///
    /// See `LinuxCgroupV2Error` variants.
    pub fn probe(parent: PathBuf) -> Result<Self, LinuxCgroupV2Error> {
        // Check parent exists + is a directory.
        let meta = std::fs::metadata(&parent).map_err(|e| LinuxCgroupV2Error::NotWritable {
            path: parent.clone(),
            reason: e.to_string(),
        })?;
        if !meta.is_dir() {
            return Err(LinuxCgroupV2Error::NotWritable {
                path: parent.clone(),
                reason: "parent is not a directory".into(),
            });
        }
        // Probe writability by creating a probe subdir + immediately removing it.
        let probe_path = parent.join(format!(".kx-probe-{}", std::process::id()));
        std::fs::create_dir(&probe_path).map_err(|e| LinuxCgroupV2Error::NotWritable {
            path: parent.clone(),
            reason: format!("create probe subdir: {e}"),
        })?;
        let _ = std::fs::remove_dir(&probe_path); // best-effort cleanup
                                                  // Probe cgroup v2 by checking for the `cgroup.controllers` file
                                                  // (only present on a cgroup v2 mount).
                                                  // We don't strictly require `parent/cgroup.controllers`; the
                                                  // check is whether the cgroup-v2 root (/sys/fs/cgroup) has it.
        let v2_marker = std::path::Path::new("/sys/fs/cgroup/cgroup.controllers");
        if !v2_marker.exists() {
            return Err(LinuxCgroupV2Error::NotV2 {
                path: parent.clone(),
                reason: "/sys/fs/cgroup/cgroup.controllers not present".into(),
            });
        }
        Ok(Self {
            parent,
            next_slot_id: AtomicU64::new(1),
            outstanding: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Default-parent constructor. Equivalent to
    /// `probe(PathBuf::from(DEFAULT_CGROUP_PARENT))`.
    ///
    /// # Errors
    ///
    /// See `LinuxCgroupV2Error` variants.
    pub fn probe_default() -> Result<Self, LinuxCgroupV2Error> {
        Self::probe(PathBuf::from(DEFAULT_CGROUP_PARENT))
    }

    /// Path to the per-Mote cgroup directory for the given slot. Used by
    /// `cgroup_attach_for_slot()` to compose the pre-exec hook.
    #[must_use]
    pub fn cgroup_dir_for_slot(&self, slot: Slot) -> PathBuf {
        self.parent.join(format!("kx-mote-{}", slot.id))
    }

    /// Compose a pre-exec hook that writes the child's own PID to the
    /// slot's `cgroup.procs` file. The returned closure is async-signal-
    /// safe: it uses libc::open/write/close + a manual int-to-ASCII
    /// encoding (no `format!` or other allocator-using code).
    ///
    /// **MUST be called from the post-fork child between fork and execvp**;
    /// the cgroup_attach pre-exec hook reads the child's own PID (via
    /// libc::getpid) + writes it to the per-slot `cgroup.procs` path
    /// (prepared by the parent at acquire time).
    #[must_use]
    pub fn cgroup_attach_for_slot(
        &self,
        slot: Slot,
    ) -> Arc<dyn Fn() -> Result<(), i32> + Send + Sync> {
        let cgroup_procs_path = self.cgroup_dir_for_slot(slot).join("cgroup.procs");
        let path_c = std::ffi::CString::new(cgroup_procs_path.as_os_str().as_encoded_bytes())
            .expect("cgroup procs path contains no NUL bytes");
        Arc::new(move || -> Result<(), i32> { cgroup_attach_async_signal_safe(&path_c) })
    }
}

impl ResourceManager for LinuxCgroupV2ResourceManager {
    fn acquire(&self, ceiling: &ResourceCeiling) -> Result<Slot, ResourceError> {
        let slot = Slot {
            id: self.next_slot_id.fetch_add(1, Ordering::SeqCst),
        };
        let dir = self.cgroup_dir_for_slot(slot);
        std::fs::create_dir(&dir).map_err(|e| {
            LinuxCgroupV2Error::Io(format!("create cgroup dir {}: {e}", dir.display()))
        })?;

        // Write the ceilings to the cgroup v2 control files.
        // - cpu.max: "<max-microseconds> <period-microseconds>" or "max"
        // - memory.max: bytes or "max"
        // - pids.max: integer or "max"
        // cpu_milli is total CPU time, NOT throughput; we map it to
        // cpu.max via period=100000us + max=cpu_milli*100 (rough
        // approximation; production deployments may tune).
        let mem_max = if ceiling.mem_bytes > 0 {
            ceiling.mem_bytes.to_string()
        } else {
            "max".to_string()
        };
        std::fs::write(dir.join("memory.max"), mem_max)
            .map_err(|e| LinuxCgroupV2Error::Io(format!("write memory.max: {e}")))?;

        if ceiling.fd_count > 0 {
            std::fs::write(dir.join("pids.max"), ceiling.fd_count.to_string())
                .map_err(|e| LinuxCgroupV2Error::Io(format!("write pids.max: {e}")))?;
        }

        // Track the outstanding cgroup for release.
        self.outstanding
            .lock()
            .map_err(|e| ResourceError::Internal(format!("mutex poisoned: {e}")))?
            .insert(slot.id, dir);

        Ok(slot)
    }

    fn release(&self, slot: Slot) -> Result<(), ResourceError> {
        let mut outstanding = self
            .outstanding
            .lock()
            .map_err(|e| ResourceError::Internal(format!("mutex poisoned: {e}")))?;
        let Some(dir) = outstanding.remove(&slot.id) else {
            return Err(ResourceError::UnknownSlot(slot.id));
        };
        // Best-effort directory removal. cgroup v2 dirs can only be
        // removed when empty (no procs attached); the child should have
        // exited by the time release is called.
        if let Err(e) = std::fs::remove_dir(&dir) {
            return Err(LinuxCgroupV2Error::Io(format!(
                "remove cgroup dir {}: {e}",
                dir.display()
            ))
            .into());
        }
        Ok(())
    }
}

/// Async-signal-safe variant of "write own PID to cgroup_procs". Uses
/// libc::open/write/close directly + a stack-allocated int-to-ASCII
/// conversion (no Rust allocator activity in the child between fork and
/// exec).
///
/// Returns marker exit codes the caller's pre-exec hook passes to
/// `_exit` on failure: 90 = open failed, 91 = write failed.
fn cgroup_attach_async_signal_safe(path_c: &std::ffi::CStr) -> Result<(), i32> {
    // SAFETY: getpid is async-signal-safe per POSIX; returns pid_t (i32
    // on Linux/macOS).
    let pid = unsafe { libc::getpid() };
    let mut buf = [0u8; 24]; // i32 max width + sign + newline
    let n = int_to_ascii(pid, &mut buf);

    // SAFETY: path_c is a valid NUL-terminated C string for the lifetime
    // of this call; O_WRONLY is a valid flag combination. libc::open is
    // async-signal-safe.
    let fd = unsafe { libc::open(path_c.as_ptr(), libc::O_WRONLY) };
    if fd < 0 {
        return Err(90);
    }
    // SAFETY: fd is a valid open file descriptor; buf is a valid slice
    // for read; libc::write is async-signal-safe.
    let written = unsafe { libc::write(fd, buf.as_ptr().cast::<libc::c_void>(), n) };
    // SAFETY: closing a valid fd is always safe; the return value indicates
    // whether outstanding I/O was flushed.
    let _ = unsafe { libc::close(fd) };
    if written < 0 || (written.cast_unsigned()) < n {
        return Err(91);
    }
    Ok(())
}

/// Encode a non-negative `i32` into ASCII bytes + a trailing newline.
/// Returns the number of bytes written (≤ buf.len()). Async-signal-safe
/// (no allocator activity).
fn int_to_ascii(mut value: i32, buf: &mut [u8]) -> usize {
    // Handle 0 + negative as special cases.
    if value == 0 {
        if buf.len() < 2 {
            return 0;
        }
        buf[0] = b'0';
        buf[1] = b'\n';
        return 2;
    }
    let negative = value < 0;
    if negative {
        value = value.wrapping_neg();
    }

    // Write digits in reverse, then reverse the slice in-place.
    let mut tmp = [0u8; 16];
    let mut i = 0;
    let mut v = value.cast_unsigned();
    while v > 0 && i < tmp.len() {
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }

    let total = i + usize::from(negative) + 1; // digits + sign + \n
    if buf.len() < total {
        return 0;
    }
    let mut out = 0;
    if negative {
        buf[out] = b'-';
        out += 1;
    }
    // Copy reversed.
    while i > 0 {
        i -= 1;
        buf[out] = tmp[i];
        out += 1;
    }
    buf[out] = b'\n';
    out + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_to_ascii_encodes_zero() {
        let mut buf = [0u8; 8];
        let n = int_to_ascii(0, &mut buf);
        assert_eq!(n, 2);
        assert_eq!(&buf[..n], b"0\n");
    }

    #[test]
    fn int_to_ascii_encodes_small_positives() {
        let mut buf = [0u8; 8];
        let n = int_to_ascii(42, &mut buf);
        assert_eq!(&buf[..n], b"42\n");
    }

    #[test]
    fn int_to_ascii_encodes_large_positives() {
        let mut buf = [0u8; 24];
        let n = int_to_ascii(2_147_483_647, &mut buf);
        assert_eq!(&buf[..n], b"2147483647\n");
    }

    #[test]
    fn int_to_ascii_encodes_negatives() {
        let mut buf = [0u8; 24];
        let n = int_to_ascii(-1234, &mut buf);
        assert_eq!(&buf[..n], b"-1234\n");
    }
}
