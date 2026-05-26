//! Small closed enums with stable `#[repr(u8)]` discriminants — wire-stable
//! across versions. [`MoteClass`] / [`ExecutorClass`] / [`FsMode`].

use kx_mote::NdClass;
use serde::{Deserialize, Serialize};

/// The non-determinism class a Mote attempts under. Mirrors [`NdClass`] from
/// `kx-mote`; restated here so the warrant layer carries its own semantically
/// equivalent enum without coupling to the journal-side discriminant.
///
/// Set by the child's role; **NOT inherited** from the parent warrant. A child
/// may be `Pure` under a `WorldMutating` parent (workers may be tighter than
/// their parent on this axis).
///
/// # Example
///
/// ```
/// use kx_warrant::MoteClass;
/// assert_eq!(MoteClass::Pure as u8, 0);
/// assert_eq!(MoteClass::ReadOnlyNondet as u8, 1);
/// assert_eq!(MoteClass::WorldMutating as u8, 2);
/// ```
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MoteClass {
    /// Pure: bit-stable function of inputs. No side effects. Safe to re-run.
    Pure = 0,
    /// Reads from a non-deterministic source (model inference, RNG) but causes
    /// no external state change. NEVER re-run once Committed.
    ReadOnlyNondet = 1,
    /// Performs an external effect (filesystem write, network call, etc.).
    /// Validate-then-commit per D20; effect-once via the broker.
    WorldMutating = 2,
}

impl MoteClass {
    /// Convert from kx-mote's `NdClass` to keep wire-format parity.
    #[inline]
    #[must_use]
    pub fn from_nd_class(nd: NdClass) -> Self {
        match nd {
            NdClass::Pure => Self::Pure,
            NdClass::ReadOnlyNondet => Self::ReadOnlyNondet,
            NdClass::WorldMutating => Self::WorldMutating,
        }
    }

    /// Convert to kx-mote's `NdClass`.
    #[inline]
    #[must_use]
    pub fn to_nd_class(self) -> NdClass {
        match self {
            Self::Pure => NdClass::Pure,
            Self::ReadOnlyNondet => NdClass::ReadOnlyNondet,
            Self::WorldMutating => NdClass::WorldMutating,
        }
    }
}

/// Which executor backend is responsible for running the Mote. Set by the
/// child's role; orthogonal to narrowing.
///
/// `Bwrap` is the OSS default on Linux; `OciDaemon` is a warrant-declared
/// opt-in for narrow cases (GPU passthrough or live-service environments);
/// `CloudMicroVm` is cloud-side per D28.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ExecutorClass {
    /// Bubblewrap over an extracted, content-addressed OCI rootfs. Daemonless;
    /// ms-spawn; least-privilege. The default on Linux.
    Bwrap = 0,
    /// Container runtime (Podman/runc preferred over Docker). Warrant-declared
    /// opt-in for narrow cases.
    OciDaemon = 1,
    /// Cloud-side microVM (firecracker / kata). Stub in OSS; concrete impl
    /// lives behind the cloud feature flag.
    CloudMicroVm = 2,
    /// macOS sandbox-exec / Seatbelt sibling of `Bwrap`. The default on macOS
    /// (the platform-conditional `default_executor()` factory picks this on
    /// `target_os = "macos"`). Compiles a `WarrantSpec` into an SBPL profile +
    /// spawns the Mote body via `posix_spawn` under `sandbox_init`-equivalent
    /// enforcement. Additive variant; existing warrant_refs are preserved
    /// because the discriminant is appended (variant 3, not interleaved).
    MacOsSandbox = 3,
}

/// Filesystem access mode for a mount in [`crate::FsScope`].
///
/// Modes form a total order under "permits at most": `ReadOnly < ReadWrite`
/// for write access; `ExecOnly` is orthogonal (permits `exec` but not
/// read/write). The intersection per path is set-intersection on the
/// permitted operations.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FsMode {
    /// Read access only.
    ReadOnly = 0,
    /// Read + write access.
    ReadWrite = 1,
    /// Execute access only (no read/write).
    ExecOnly = 2,
}

impl FsMode {
    /// `true` iff `self` is no wider than `parent` on every operation.
    #[inline]
    #[must_use]
    pub fn is_subset_of(self, parent: FsMode) -> bool {
        matches!(
            (self, parent),
            (Self::ReadOnly, Self::ReadOnly | Self::ReadWrite)
                | (Self::ReadWrite, Self::ReadWrite)
                | (Self::ExecOnly, Self::ExecOnly)
        )
    }
}
