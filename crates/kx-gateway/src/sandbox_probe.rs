//! What this host's sandbox can actually enforce — and what it can only be told.
//!
//! A registered script declares a ceiling: a filesystem scope, an egress allowlist, a
//! memory limit, a wall-clock budget, an output cap. The runtime carries that declaration
//! all the way to a real sandboxed spawn. What it did NOT do was tell anyone which of
//! those axes the host underneath is capable of honouring — and they are not all the same
//! on every platform. An axis that is declared, carried, and then quietly ignored is worse
//! than an axis that does not exist: it reads, at every layer above it, exactly like a
//! constraint that is being applied.
//!
//! The concrete case this exists for: `net_hosts` on Linux. Confining egress to named
//! hosts needs a parent-side firewall, which needs privileges the runtime does not have,
//! so `bwrap` is given no network flag at all — and a script that asked to reach ONE host
//! runs with the network wide open. It was accepted, it ran, and nothing said so.
//!
//! So this module answers two questions, and the second is the one that matters:
//!
//! 1. **What can this host enforce?** [`probe`] reports every axis as
//!    [`AxisSupport::Enforced`], [`AxisSupport::Refused`], or
//!    [`AxisSupport::DeclaredNotEnforced`].
//! 2. **What must therefore be refused?** [`unenforceable_wish`] turns that report into a
//!    registration-time refusal, so a declaration the host cannot honour fails when it is
//!    made rather than appearing to hold forever.
//!
//! The report is derived from the executor class and the build target, not measured by
//! trying it. That is a deliberate limit and it is stated rather than hidden: this
//! describes what the runtime's own spawn path is written to do, which is exactly the
//! thing a reader cannot otherwise discover without reading it.

use std::fmt;

use kx_warrant::ExecutorClass;

/// How well one axis of a declared ceiling survives contact with this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSupport {
    /// The host applies it. A declaration on this axis is a constraint.
    Enforced,
    /// The host cannot apply it, and says so: a declaration is REFUSED rather than
    /// accepted and ignored. Fail-closed, and the honest answer when the alternative is
    /// pretending.
    Refused,
    /// The host does not apply it and nothing refuses it. This is the dangerous state —
    /// the declaration is carried, looks authoritative everywhere it is displayed, and
    /// constrains nothing. Every entry here is a bug with a name.
    DeclaredNotEnforced,
}

impl fmt::Display for AxisSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AxisSupport::Enforced => "enforced",
            AxisSupport::Refused => "refused (the host cannot confine this axis)",
            AxisSupport::DeclaredNotEnforced => "DECLARED BUT NOT ENFORCED",
        })
    }
}

/// One axis, its support, and why.
#[derive(Debug, Clone)]
pub struct AxisReport {
    /// The axis name, as a declaration spells it.
    pub axis: &'static str,
    /// What this host does with it.
    pub support: AxisSupport,
    /// The mechanism, or the reason there is none.
    pub detail: &'static str,
}

/// What the running host can enforce for sandboxed script bodies.
#[derive(Debug, Clone)]
pub struct SandboxCapabilities {
    /// The executor class the report describes.
    pub executor: &'static str,
    /// The platform the runtime was built for.
    pub platform: &'static str,
    /// Every axis, in a stable order.
    pub axes: Vec<AxisReport>,
}

impl SandboxCapabilities {
    /// The axes this host accepts and then ignores. Empty is the goal; non-empty is a
    /// list of things no one should rely on.
    #[must_use]
    pub fn unenforced(&self) -> Vec<&AxisReport> {
        self.axes
            .iter()
            .filter(|a| a.support == AxisSupport::DeclaredNotEnforced)
            .collect()
    }

    /// Look one axis up.
    #[must_use]
    pub fn axis(&self, name: &str) -> Option<&AxisReport> {
        self.axes.iter().find(|a| a.axis == name)
    }
}

/// Can THIS platform hold a hosted dev server confined? The requirements differ
/// from a script's: the server LISTENS on loopback (inbound + bind, which the
/// macOS profile language expresses and bwrap cannot — `--unshare-net` would
/// sever the very port the gateway must reach), and it forks freely (macOS can
/// GRANT fork explicitly; bwrap has no pid-limit axis but shares the verdict
/// via the loopback refusal first). `Err` carries the reason a refusal/status
/// detail surfaces verbatim.
pub fn hosted_confinement() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        if std::path::Path::new("/usr/bin/sandbox-exec").is_file() {
            Ok(())
        } else {
            Err("the platform sandbox binary (/usr/bin/sandbox-exec) is missing".into())
        }
    } else {
        Err(
            "bwrap cannot confine a loopback-listening server (per-host egress and inbound \
             loopback are unenforceable; --unshare-net would sever the served port)"
                .into(),
        )
    }
}

/// Report what `exec_class` can enforce on this build's target platform.
#[must_use]
pub fn probe(exec_class: ExecutorClass) -> SandboxCapabilities {
    let mut axes = confinement_axes(exec_class);
    axes.extend(ceiling_axes(exec_class));
    axes.sort_by_key(|a| a.axis);
    SandboxCapabilities {
        executor: match exec_class {
            ExecutorClass::Bwrap => "bwrap",
            ExecutorClass::MacOsSandbox => "macos-sandbox",
            _ => "other",
        },
        platform: std::env::consts::OS,
        axes,
    }
}

/// The axes that confine what a body can REACH — filesystem and network.
fn confinement_axes(exec_class: ExecutorClass) -> Vec<AxisReport> {
    let bwrap = matches!(exec_class, ExecutorClass::Bwrap);
    let macos = matches!(exec_class, ExecutorClass::MacOsSandbox);
    vec![
        AxisReport {
            axis: "fs_mounts",
            support: if bwrap || macos {
                AxisSupport::Enforced
            } else {
                AxisSupport::DeclaredNotEnforced
            },
            detail: if bwrap {
                "bind mounts, read-only unless the mode says otherwise"
            } else {
                "a deny-by-default profile with per-path subpath rules"
            },
        },
        AxisReport {
            axis: "net_hosts (empty — no egress)",
            support: AxisSupport::Enforced,
            detail: if bwrap {
                "a network namespace of its own (--unshare-net)"
            } else {
                "the profile denies by default and emits no outbound rule"
            },
        },
        AxisReport {
            // The axis this module exists for — and it splits by host, not by platform.
            axis: "net_hosts (loopback only)",
            support: if macos {
                AxisSupport::Enforced
            } else {
                AxisSupport::Refused
            },
            detail: if macos {
                "the profile emits a loopback-only outbound rule, so a loopback \
                 allowlist is a real confinement"
            } else {
                "confining egress needs a parent-side firewall and the privileges to \
                 install one; without one the namespace is left un-isolated, so this is \
                 refused rather than run with the network wide open"
            },
        },
        AxisReport {
            axis: "net_hosts (any other host)",
            support: AxisSupport::Refused,
            detail: if bwrap {
                "confining egress per host needs a parent-side firewall and the \
                 privileges to install one, so a declaration is refused rather than \
                 accepted and run with the network wide open"
            } else {
                "the profile can express loopback or unrestricted and nothing between, \
                 and granting unrestricted would exceed what the script declared"
            },
        },
    ]
}

/// Whether every host in a wish is a loopback name this platform can actually confine to.
/// Empty is vacuously true and is handled by the caller as "no egress at all".
fn all_loopback<'a>(hosts: impl IntoIterator<Item = &'a str>) -> bool {
    hosts
        .into_iter()
        .all(|h| matches!(h, "localhost" | "127.0.0.1" | "::1"))
}

/// The axes that bound how much a body may CONSUME, plus the two that nothing bounds.
fn ceiling_axes(exec_class: ExecutorClass) -> Vec<AxisReport> {
    let bwrap = matches!(exec_class, ExecutorClass::Bwrap);
    vec![
        AxisReport {
            axis: "wall_clock_ms",
            support: AxisSupport::Enforced,
            detail: "the shim signals its own process group, and the host applies an \
                     outer deadline behind it",
        },
        AxisReport {
            axis: "max_output_bytes",
            support: AxisSupport::Enforced,
            detail: "the shim REFUSES a run that exceeds the cap rather than truncating \
                     it — a truncated result would read as a complete one",
        },
        AxisReport {
            axis: "mem_bytes",
            support: if bwrap {
                AxisSupport::Enforced
            } else {
                AxisSupport::Refused
            },
            detail: if bwrap {
                "an address-space rlimit, applied by the host and again by the shim"
            } else {
                "this platform rejects the address-space rlimit, so the shim cannot \
                 apply one and a declaration is refused rather than run unbounded"
            },
        },
        AxisReport {
            axis: "cpu_milli / fd_count / disk_bytes",
            support: if bwrap {
                AxisSupport::Enforced
            } else {
                AxisSupport::DeclaredNotEnforced
            },
            detail: if bwrap {
                "rlimits applied around the spawn"
            } else {
                "the script path spawns the platform sandbox directly and applies no \
                 rlimits, so these are carried and ignored — no script declares them \
                 today, which is why nothing has broken"
            },
        },
        AxisReport {
            // True on both platforms, and stated because it is the axis most likely to
            // be assumed. A sandbox that confines the filesystem is easy to read as one
            // that confines everything.
            axis: "process count / fork",
            support: AxisSupport::DeclaredNotEnforced,
            detail: "nothing bounds how many processes a body may spawn: there is no \
                     process rlimit on this path and no pid namespace, and the platform \
                     profile must permit fork at all for the interpreter to start",
        },
        AxisReport {
            axis: "syscall_profile_ref",
            support: AxisSupport::DeclaredNotEnforced,
            detail: "the reference is recorded for audit and never resolved into a \
                     filter; the empty sentinel is what lets tools share one union grant",
        },
    ]
}

/// The reason a declared wish cannot be honoured here, or `None` when it can.
///
/// This is the enforcement half. A report nobody consults is a document; turning it into
/// a refusal at registration is what stops an unenforceable declaration from being
/// accepted, displayed as a constraint, and relied on.
///
/// Deliberately conservative about WHICH axes refuse. An axis in
/// [`AxisSupport::DeclaredNotEnforced`] is not grounds to refuse a script that never
/// mentioned it — refusing on `fork` would refuse every script ever written, since no
/// declaration can opt out of a bound the platform does not offer. What is refused is a
/// declaration the caller actually MADE and the host cannot keep.
#[must_use]
pub fn unenforceable_wish<'a>(
    exec_class: ExecutorClass,
    net_hosts: impl IntoIterator<Item = &'a str>,
    wants_mem_ceiling: bool,
) -> Option<String> {
    let caps = probe(exec_class);
    let hosts: Vec<&str> = net_hosts.into_iter().collect();
    if !hosts.is_empty() {
        // Which axis applies depends on WHAT was asked for, not just on the platform.
        // macOS can genuinely confine a body to loopback; it cannot express anything
        // narrower than "unrestricted" for any other host, and granting unrestricted
        // would hand the script more than it declared.
        let axis = if all_loopback(hosts.iter().copied()) {
            "net_hosts (loopback only)"
        } else {
            "net_hosts (any other host)"
        };
        if let Some(a) = caps.axis(axis) {
            if a.support != AxisSupport::Enforced {
                return Some(format!(
                    "this host cannot confine egress to {hosts:?} — {}. Declare no hosts \
                     to run with no network at all",
                    a.detail
                ));
            }
        }
    }
    if wants_mem_ceiling {
        if let Some(a) = caps.axis("mem_bytes") {
            if a.support != AxisSupport::Enforced {
                return Some(format!(
                    "this host cannot apply a memory ceiling — {}. Declare 0 to run \
                     without one",
                    a.detail
                ));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report must not claim more than the spawn path delivers. These are the axes
    /// the code genuinely does not apply, and the test exists so that FIXING one of them
    /// forces this report to be updated with it — a capability report that drifts out of
    /// date is the same failure it was written to prevent.
    #[test]
    fn the_report_names_what_is_not_enforced() {
        for class in [ExecutorClass::Bwrap, ExecutorClass::MacOsSandbox] {
            let caps = probe(class);
            let unenforced: Vec<&str> = caps.unenforced().iter().map(|a| a.axis).collect();
            assert!(
                unenforced.contains(&"process count / fork"),
                "{class:?}: nothing bounds process count on either platform"
            );
            assert!(
                unenforced.contains(&"syscall_profile_ref"),
                "{class:?}: the syscall profile is recorded, never resolved"
            );
        }
        // macOS scripts bypass the rlimit path entirely; Linux does not.
        assert_eq!(
            probe(ExecutorClass::MacOsSandbox)
                .axis("cpu_milli / fd_count / disk_bytes")
                .map(|a| a.support),
            Some(AxisSupport::DeclaredNotEnforced)
        );
        assert_eq!(
            probe(ExecutorClass::Bwrap)
                .axis("cpu_milli / fd_count / disk_bytes")
                .map(|a| a.support),
            Some(AxisSupport::Enforced)
        );
    }

    /// No egress at all is enforced everywhere, and it must stay distinguishable from an
    /// allowlist — otherwise "the sandbox handles network" would be true in the report
    /// and false in the half that matters.
    #[test]
    fn no_egress_is_enforced_everywhere() {
        for class in [ExecutorClass::Bwrap, ExecutorClass::MacOsSandbox] {
            assert_eq!(
                probe(class)
                    .axis("net_hosts (empty — no egress)")
                    .map(|a| a.support),
                Some(AxisSupport::Enforced),
                "{class:?}"
            );
        }
    }

    /// The egress split is by HOST, not by platform, and getting that wrong in either
    /// direction is costly. Too permissive and Linux keeps running scripts with the
    /// network open while claiming otherwise; too strict and macOS stops accepting a
    /// loopback confinement it genuinely applies — which would not fail loudly, it would
    /// turn the existing egress test into a silent skip.
    #[test]
    fn loopback_is_confinable_on_macos_and_nothing_is_on_bwrap() {
        let mac = ExecutorClass::MacOsSandbox;
        assert!(
            unenforceable_wish(mac, ["127.0.0.1"], false).is_none(),
            "macOS emits a loopback-only outbound rule; refusing this would be wrong"
        );
        assert!(
            unenforceable_wish(mac, ["localhost", "::1"], false).is_none(),
            "every spelling of loopback the profile writer accepts"
        );
        assert!(
            unenforceable_wish(mac, ["api.example.com"], false).is_some(),
            "and anything else has no expressible rule between loopback and unrestricted"
        );
        assert!(
            unenforceable_wish(mac, ["127.0.0.1", "api.example.com"], false).is_some(),
            "one unconfinable host in the set is enough — the wish is not partly granted"
        );

        let linux = ExecutorClass::Bwrap;
        assert!(
            unenforceable_wish(linux, ["127.0.0.1"], false).is_some(),
            "bwrap has no per-host mechanism at all, loopback included"
        );
        assert!(unenforceable_wish(linux, ["api.example.com"], false).is_some(),);
    }

    /// The refusal fires on the axes a caller DECLARED, and stays silent otherwise. A
    /// script that asks for nothing unenforceable must keep registering exactly as before.
    #[test]
    fn only_a_declared_unenforceable_axis_refuses() {
        for class in [ExecutorClass::Bwrap, ExecutorClass::MacOsSandbox] {
            assert!(
                unenforceable_wish(class, [], false).is_none(),
                "{class:?}: a script declaring neither must not be refused"
            );
        }
        assert!(
            unenforceable_wish(ExecutorClass::MacOsSandbox, [], true).is_some(),
            "a memory ceiling this platform cannot apply is refused when it is asked for"
        );
        assert!(
            unenforceable_wish(ExecutorClass::Bwrap, [], true).is_none(),
            "and accepted where it genuinely holds"
        );
    }

    /// The refusal has to say what to do instead. A fail-closed message that only says
    /// "no" turns a fixable declaration into a dead end.
    #[test]
    fn a_refusal_says_what_would_work() {
        let egress = unenforceable_wish(ExecutorClass::Bwrap, ["127.0.0.1"], false).unwrap();
        assert!(egress.contains("Declare no hosts"), "{egress}");
        let mem = unenforceable_wish(ExecutorClass::MacOsSandbox, [], true).unwrap();
        assert!(mem.contains("Declare 0"), "{mem}");
    }
}
