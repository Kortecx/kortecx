//! Application-layer egress policy for the HTTP transport (M5.2b, D80/D94).
//!
//! The broker already gates `request.net_scope ⊆ warrant.net_scope` declaratively
//! (`LocalCapabilityBroker::precheck`). This module is the **second, behavioural**
//! half: it binds the host the [`crate::HttpTransport`] is *allowed* to dial to the
//! host it *actually* resolves+connects to, and refuses the SSRF / DNS-rebind /
//! cloud-metadata vectors that a declarative host check alone cannot see.
//!
//! It is deliberately **pure** (zero I/O): [`classify_ip`] + [`EgressPolicy`] +
//! [`vet_resolved_addr`] are total functions over their inputs, so the security
//! decision surface is unit-testable without a socket. The transport's
//! `VettingResolver` is the only place that performs DNS and then calls
//! [`vet_resolved_addr`] on each resolved address.
//!
//! ## Honest OSS boundary (D94, risk #8)
//!
//! This is an **application-layer** control. A *compromised in-process tool* could
//! issue its own syscalls and bypass this resolver entirely; only kernel-level
//! egress isolation (`bwrap` + nftables, NET_ADMIN — a `kx-cloud` concern) closes
//! that. The OSS guarantee is: the runtime's own HTTP path never dials a host the
//! warrant did not grant, never follows a cross-host redirect, and never reaches a
//! private/loopback/link-local address via a public hostname (rebind defense).

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use kx_warrant::NetScope;

/// Classification of a resolved IP address for SSRF / rebind defense.
///
/// Every variant except [`IpClass::Public`] is an address that must not be reached
/// *implicitly* (via a public hostname). It may be reached only when the operator
/// allowlisted that exact literal address — see [`vet_resolved_addr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpClass {
    /// A globally-routable address — reachable when its host is allowlisted.
    Public,
    /// `127.0.0.0/8`, `::1`.
    Loopback,
    /// `169.254.0.0/16` (**includes the `169.254.169.254` cloud-metadata IP**),
    /// `fe80::/10`.
    LinkLocal,
    /// `10/8`, `172.16/12`, `192.168/16`, `fc00::/7` (IPv6 ULA).
    Private,
    /// `0.0.0.0`, `::`.
    Unspecified,
    /// `224.0.0.0/4`, `ff00::/8`.
    Multicast,
}

impl IpClass {
    /// True for every class that must not be reached implicitly (anything but
    /// [`IpClass::Public`]).
    #[must_use]
    pub fn is_non_public(self) -> bool {
        !matches!(self, IpClass::Public)
    }
}

/// Classify `ip` into an [`IpClass`].
///
/// Ranges are matched **by raw octets** (not the unstable `std::net` `is_*` family)
/// so classification is identical across toolchains. IPv4-mapped IPv6
/// (`::ffff:a.b.c.d`) is un-mapped first so a mapped loopback/private address is
/// classified by its embedded IPv4 (a mapped `::ffff:127.0.0.1` is loopback, never
/// public).
#[must_use]
pub fn classify_ip(ip: &IpAddr) -> IpClass {
    match ip {
        IpAddr::V4(v4) => classify_v4(*v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(v4: Ipv4Addr) -> IpClass {
    let o = v4.octets();
    match o {
        [0, 0, 0, 0] => IpClass::Unspecified,
        [127, ..] => IpClass::Loopback,
        // 169.254/16 link-local — INCLUDES 169.254.169.254 (cloud metadata).
        [169, 254, ..] => IpClass::LinkLocal,
        // 10/8, 192.168/16, and 172.16/12 (the guarded second octet).
        [10, ..] | [192, 168, ..] => IpClass::Private,
        [172, b, ..] if (16..=31).contains(&b) => IpClass::Private,
        // 224.0.0.0/4 multicast (224–239 in the first octet).
        [a, ..] if (224..=239).contains(&a) => IpClass::Multicast,
        _ => IpClass::Public,
    }
}

fn classify_v6(v6: &Ipv6Addr) -> IpClass {
    // Un-map `::ffff:a.b.c.d` so a mapped IPv4 is classified by its embedded form.
    // (`to_ipv4_mapped` matches ONLY `::ffff:x.x.x.x`, never `::1`/`::`, so it does
    // not mis-fold loopback/unspecified.)
    if let Some(v4) = v6.to_ipv4_mapped() {
        return classify_v4(v4);
    }
    let o = v6.octets();
    if o == [0; 16] {
        return IpClass::Unspecified; // ::
    }
    if o == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1] {
        return IpClass::Loopback; // ::1
    }
    if o[0] == 0xff {
        return IpClass::Multicast; // ff00::/8
    }
    if (o[0] & 0xfe) == 0xfc {
        return IpClass::Private; // fc00::/7 (unique-local)
    }
    if o[0] == 0xfe && (o[1] & 0xc0) == 0x80 {
        return IpClass::LinkLocal; // fe80::/10
    }
    IpClass::Public
}

/// The set of hosts the [`crate::HttpTransport`] may dial, derived from the
/// warrant-validated `net_scope` of the resolved tool.
///
/// Host matching is **host-only** (no port, no scheme) and case-insensitive, to
/// agree with `kx_warrant::NetScope`'s host-set `is_subset_of` semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressPolicy {
    allowed_hosts: BTreeSet<String>,
}

impl EgressPolicy {
    /// Build a policy from the granted `net_scope`. `NetScope::None` yields an
    /// **empty** policy (every dial refused — fail-closed); an allowlist yields the
    /// normalized host set.
    #[must_use]
    pub fn from_net_scope(scope: &NetScope) -> Self {
        let allowed_hosts = match scope {
            NetScope::None => BTreeSet::new(),
            NetScope::EgressAllowlist(hosts) => {
                hosts.iter().map(|h| normalize_host(&h.0)).collect()
            }
        };
        Self { allowed_hosts }
    }

    /// True iff `host` (normalized) is explicitly allowlisted.
    #[must_use]
    pub fn permits_host(&self, host: &str) -> bool {
        self.allowed_hosts.contains(&normalize_host(host))
    }

    /// True iff this policy permits no egress at all (a `NetScope::None` tool).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allowed_hosts.is_empty()
    }
}

/// Normalize a host for comparison: trim, lowercase, strip a trailing FQDN dot and
/// surrounding IPv6 brackets, so `Example.COM`, `example.com.`, and `[::1]`/`::1`
/// compare equal to their canonical form.
fn normalize_host(host: &str) -> String {
    let h = host.trim().trim_end_matches('.');
    let h = h
        .strip_prefix('[')
        .map_or(h, |rest| rest.strip_suffix(']').unwrap_or(rest));
    h.to_ascii_lowercase()
}

/// Why a resolved address was refused egress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EgressDenied {
    /// The host is not in the warrant-derived allowlist.
    HostNotAllowed {
        /// The refused host (safe to log — never a secret).
        host: String,
    },
    /// The host resolved to a non-public address (SSRF / DNS-rebind / metadata)
    /// and was **not** an explicitly-allowlisted private literal.
    PrivateAddressRefused {
        /// The host whose resolution was refused.
        host: String,
        /// The class of the refused address.
        class: IpClass,
    },
}

/// Vet a resolved `addr` for `host` against `policy`. `Ok(())` ⇒ the transport may
/// dial it; `Err` ⇒ refuse.
///
/// The rule (rebind / SSRF defense): a non-public address is refused **unless the
/// host is a literal IP equal to that address** — i.e. the operator explicitly
/// allowlisted that private/loopback literal. A *public hostname* that
/// resolves (or rebinds) to a private/loopback/link-local address never parses as
/// an IP literal, so it is always refused — a public name can never reach the
/// metadata endpoint or an internal host through this transport.
///
/// # Errors
///
/// [`EgressDenied::HostNotAllowed`] if `host` is not allowlisted;
/// [`EgressDenied::PrivateAddressRefused`] if it resolves to a non-public address
/// it was not explicitly allowed to reach.
pub fn vet_resolved_addr(
    host: &str,
    addr: &SocketAddr,
    policy: &EgressPolicy,
) -> Result<(), EgressDenied> {
    if !policy.permits_host(host) {
        return Err(EgressDenied::HostNotAllowed {
            host: host.to_string(),
        });
    }
    let class = classify_ip(&addr.ip());
    if class == IpClass::Public {
        return Ok(());
    }
    // Non-public: permit ONLY when the host is a literal IP equal to this address
    // (an explicit operator opt-in, e.g. an allowlisted `127.0.0.1`). A DNS name
    // that resolved here does not parse as an IP literal ⇒ refused (rebind/SSRF).
    if let Ok(host_ip) = host.parse::<IpAddr>() {
        if host_ip == addr.ip() {
            return Ok(());
        }
    }
    Err(EgressDenied::PrivateAddressRefused {
        host: host.to_string(),
        class,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_warrant::Host;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn classify_ipv4_ranges() {
        assert_eq!(classify_ip(&v4(0, 0, 0, 0)), IpClass::Unspecified);
        assert_eq!(classify_ip(&v4(127, 0, 0, 1)), IpClass::Loopback);
        assert_eq!(classify_ip(&v4(127, 13, 9, 2)), IpClass::Loopback);
        // The cloud-metadata endpoint MUST classify link-local.
        assert_eq!(classify_ip(&v4(169, 254, 169, 254)), IpClass::LinkLocal);
        assert_eq!(classify_ip(&v4(169, 254, 1, 1)), IpClass::LinkLocal);
        assert_eq!(classify_ip(&v4(10, 0, 0, 1)), IpClass::Private);
        assert_eq!(classify_ip(&v4(172, 16, 0, 1)), IpClass::Private);
        assert_eq!(classify_ip(&v4(172, 31, 255, 255)), IpClass::Private);
        // 172.32 is OUTSIDE the private block — public.
        assert_eq!(classify_ip(&v4(172, 32, 0, 1)), IpClass::Public);
        assert_eq!(classify_ip(&v4(172, 15, 0, 1)), IpClass::Public);
        assert_eq!(classify_ip(&v4(192, 168, 1, 1)), IpClass::Private);
        assert_eq!(classify_ip(&v4(192, 169, 1, 1)), IpClass::Public);
        assert_eq!(classify_ip(&v4(224, 0, 0, 1)), IpClass::Multicast);
        assert_eq!(classify_ip(&v4(239, 255, 255, 255)), IpClass::Multicast);
        assert_eq!(classify_ip(&v4(8, 8, 8, 8)), IpClass::Public);
        assert_eq!(classify_ip(&v4(1, 1, 1, 1)), IpClass::Public);
    }

    #[test]
    fn classify_ipv6_ranges() {
        assert_eq!(
            classify_ip(&IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
            IpClass::Unspecified
        );
        assert_eq!(
            classify_ip(&IpAddr::V6(Ipv6Addr::LOCALHOST)),
            IpClass::Loopback
        );
        assert_eq!(
            classify_ip(&IpAddr::V6("fe80::1".parse().unwrap())),
            IpClass::LinkLocal
        );
        assert_eq!(
            classify_ip(&IpAddr::V6("fc00::1".parse().unwrap())),
            IpClass::Private
        );
        assert_eq!(
            classify_ip(&IpAddr::V6("fd12:3456::1".parse().unwrap())),
            IpClass::Private
        );
        assert_eq!(
            classify_ip(&IpAddr::V6("ff02::1".parse().unwrap())),
            IpClass::Multicast
        );
        assert_eq!(
            classify_ip(&IpAddr::V6("2606:4700::1111".parse().unwrap())),
            IpClass::Public
        );
        // IPv4-mapped loopback must un-map to loopback, never public.
        assert_eq!(
            classify_ip(&IpAddr::V6("::ffff:127.0.0.1".parse().unwrap())),
            IpClass::Loopback
        );
        assert_eq!(
            classify_ip(&IpAddr::V6("::ffff:169.254.169.254".parse().unwrap())),
            IpClass::LinkLocal
        );
        assert_eq!(
            classify_ip(&IpAddr::V6("::ffff:8.8.8.8".parse().unwrap())),
            IpClass::Public
        );
    }

    fn policy(hosts: &[&str]) -> EgressPolicy {
        let set: BTreeSet<Host> = hosts.iter().map(|h| Host((*h).to_string())).collect();
        EgressPolicy::from_net_scope(&NetScope::EgressAllowlist(set))
    }

    #[test]
    fn none_scope_is_empty_policy() {
        let p = EgressPolicy::from_net_scope(&NetScope::None);
        assert!(p.is_empty());
        assert!(!p.permits_host("example.com"));
    }

    #[test]
    fn permits_host_is_case_and_dot_and_bracket_insensitive() {
        let p = policy(&["Example.COM", "127.0.0.1"]);
        assert!(p.permits_host("example.com"));
        assert!(p.permits_host("EXAMPLE.com."));
        assert!(p.permits_host("127.0.0.1"));
        assert!(!p.permits_host("evil.com"));
        // Bracketed IPv6 literal normalizes to its bare form.
        let p6 = policy(&["::1"]);
        assert!(p6.permits_host("[::1]"));
        assert!(p6.permits_host("::1"));
    }

    #[test]
    fn vet_allows_public_allowlisted_host() {
        let p = policy(&["api.example.com"]);
        let addr = SocketAddr::new(v4(93, 184, 216, 34), 443);
        assert_eq!(vet_resolved_addr("api.example.com", &addr, &p), Ok(()));
    }

    #[test]
    fn vet_refuses_unallowlisted_host() {
        let p = policy(&["api.example.com"]);
        let addr = SocketAddr::new(v4(93, 184, 216, 34), 443);
        assert!(matches!(
            vet_resolved_addr("evil.com", &addr, &p),
            Err(EgressDenied::HostNotAllowed { .. })
        ));
    }

    #[test]
    fn vet_refuses_public_host_rebinding_to_private() {
        // The killer case: an allowlisted PUBLIC hostname that resolves/rebinds to a
        // private or metadata IP. The host is not an IP literal ⇒ refused.
        let p = policy(&["api.example.com"]);
        for ip in [
            v4(169, 254, 169, 254), // cloud metadata
            v4(127, 0, 0, 1),
            v4(10, 0, 0, 5),
            v4(192, 168, 1, 1),
        ] {
            let addr = SocketAddr::new(ip, 80);
            assert!(
                matches!(
                    vet_resolved_addr("api.example.com", &addr, &p),
                    Err(EgressDenied::PrivateAddressRefused { .. })
                ),
                "rebind to {ip} must be refused"
            );
        }
    }

    #[test]
    fn vet_allows_explicit_private_literal() {
        // An operator may explicitly allowlist a private/loopback LITERAL (e.g. a
        // hermetic test server, or an on-prem internal host by IP).
        let p = policy(&["127.0.0.1"]);
        let addr = SocketAddr::new(v4(127, 0, 0, 1), 8080);
        assert_eq!(vet_resolved_addr("127.0.0.1", &addr, &p), Ok(()));
    }

    #[test]
    fn vet_refuses_private_literal_resolving_elsewhere() {
        // A literal that (somehow) resolves to a DIFFERENT private IP than itself is
        // refused — the opt-in is for the exact address only.
        let p = policy(&["127.0.0.1"]);
        let addr = SocketAddr::new(v4(10, 0, 0, 1), 8080);
        assert!(matches!(
            vet_resolved_addr("127.0.0.1", &addr, &p),
            Err(EgressDenied::PrivateAddressRefused { .. })
        ));
    }
}
