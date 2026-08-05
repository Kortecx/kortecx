#!/bin/sh
# kortecx `kx` installer — the FFI-free runtime, no toolchain required.
#
#   curl -fsSL https://raw.githubusercontent.com/Kortecx/kortecx/main/scripts/install.sh | sh
#
# Detects your platform, downloads a SHA-256-verified prebuilt `kx` from the pinned
# GitHub Release, and installs it to ${KX_INSTALL_DIR:-$HOME/.local/bin} — together
# with the kx-tools bundle (the bundled agent tool binaries: kx-mcp-echo/calc/kv and
# the four kx-connector-* sidecars), placed BESIDE `kx` where the runtime resolves
# them. No sudo, no C++ toolchain, idempotent, fail-closed. POSIX sh.
#
# Env:
#   KX_VERSION       release tag to install (default: latest)
#   KX_INSTALL_DIR   install dir (default: $HOME/.local/bin)
#   KX_BASE_URL      override the download base URL (default: the GitHub Release;
#                    the seam that lets the install path be verified against a
#                    locally-served build before a tag exists)
#   KX_SKIP_TOOLS    1/true/yes/on skips the kx-tools bundle (the agent verbs then
#                    refuse with an actionable message at run time); 0/false/no/off
#                    and unset install it
set -eu

REPO="Kortecx/kortecx"
KX_VERSION="${KX_VERSION:-latest}"
KX_INSTALL_DIR="${KX_INSTALL_DIR:-$HOME/.local/bin}"
KX_SKIP_TOOLS="${KX_SKIP_TOOLS:-}"

say() { printf '%s\n' "$*"; }
err() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

# --- 1. Detect the target triple --------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Linux)
        case "$arch" in
            x86_64 | amd64) triple="x86_64-unknown-linux-gnu" ;;
            aarch64 | arm64) triple="aarch64-unknown-linux-gnu" ;;
            *) err "unsupported Linux arch: $arch (prebuilt: x86_64, aarch64)" ;;
        esac
        ;;
    Darwin)
        case "$arch" in
            arm64 | aarch64) triple="aarch64-apple-darwin" ;;
            # Forward seam: an Intel-mac prebuilt (x86_64-apple-darwin) is not
            # published yet — build from source (`just setup`) or use Apple Silicon.
            x86_64) err "macOS x86_64 prebuilt not published yet — build from source (just setup)" ;;
            *) err "unsupported macOS arch: $arch" ;;
        esac
        ;;
    # Forward seam: Windows (x86_64-pc-windows-msvc) is not published yet.
    MSYS_NT* | CYGWIN* | MINGW*)
        err "Windows prebuilt not published yet — use WSL, or build from source (just setup)" ;;
    *) err "unsupported OS: $os" ;;
esac

# --- 2. Download tooling -----------------------------------------------------
if command -v curl >/dev/null 2>&1; then
    dl() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    dl() { wget -q "$1" -O "$2"; }
else
    err "need curl or wget on PATH"
fi
if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    err "need sha256sum or shasum on PATH"
fi

# --- 3. Resolve the release URL ---------------------------------------------
if [ "$KX_VERSION" = "latest" ]; then
    base="https://github.com/$REPO/releases/latest/download"
else
    base="https://github.com/$REPO/releases/download/$KX_VERSION"
fi
# The verification seam: a pre-tag proof serves a locally-built dist and points
# here. The default above is the production path and stays byte-identical.
base="${KX_BASE_URL:-$base}"
asset="kx-$triple"
tools_asset="kx-tools-$triple.tar.gz"
say "kortecx installer — $triple (version: $KX_VERSION)"

# --- 4. Download + SHA-256 verify (atomic) ----------------------------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
dl "$base/$asset" "$tmp/$asset.partial" || err "download failed: $base/$asset"
dl "$base/checksums.txt" "$tmp/checksums.txt" || err "download failed: $base/checksums.txt"

want="$(awk -v a="$asset" '$2 == a {print $1}' "$tmp/checksums.txt")"
[ -n "$want" ] || err "no checksum for $asset in checksums.txt"
got="$(sha256 "$tmp/$asset.partial")"
[ "$got" = "$want" ] || err "SHA-256 mismatch for $asset: expected $want, got $got"

# --- 5. Install --------------------------------------------------------------
mkdir -p "$KX_INSTALL_DIR"
chmod +x "$tmp/$asset.partial"
mv -f "$tmp/$asset.partial" "$KX_INSTALL_DIR/kx"
say " ✓ installed kx → $KX_INSTALL_DIR/kx  (sha256 $got)"

# --- 6. The kx-tools bundle (the agent's own tool binaries) -------------------
# The runtime seeds its agent recipes only when the bundled tool binaries resolve
# BESIDE `kx`, so this step is what makes `kx chat --tools`/`kx agent run` work
# on an installed box. The failure posture is discriminated by checksums.txt,
# which is this release's own manifest:
#   - no tools row     -> the release predates the bundle: say so loudly, continue
#                         (this script is served from `main` while assets come
#                         from the pinned release, so the two can legitimately
#                         differ around the first bundle-shipping tag);
#   - row present but  -> HARD ERROR. A broken new release must never
#     fetch/verify fails    half-install silently.
tools_want="$(awk -v a="$tools_asset" '$2 == a {print $1}' "$tmp/checksums.txt")"
# Truthiness, not mere non-emptiness: KX_SKIP_TOOLS=0 is a user spelling "do not
# skip", and treating it as "skip" would silently withhold the agent tools.
skip_tools=no
case "$KX_SKIP_TOOLS" in
    1 | true | TRUE | yes | YES | on | ON) skip_tools=yes ;;
    "" | 0 | false | FALSE | no | NO | off | OFF) skip_tools=no ;;
    *) err "KX_SKIP_TOOLS must be 1/true/yes/on or 0/false/no/off, got: $KX_SKIP_TOOLS" ;;
esac
if [ "$skip_tools" = yes ]; then
    say " · KX_SKIP_TOOLS=$KX_SKIP_TOOLS — skipping the kx-tools bundle (agent verbs will refuse until it is installed)"
elif [ -z "$tools_want" ]; then
    say ""
    say " ⚠ this release ($KX_VERSION) has no $tools_asset asset — the bundled agent tools"
    say "   (kx-mcp-echo/calc/kv and the four kx-connector-* sidecars) are NOT installed, so"
    say "   the agent verbs will refuse. Re-run against a newer release when available."
else
    command -v tar >/dev/null 2>&1 || err "need tar on PATH to unpack $tools_asset"
    dl "$base/$tools_asset" "$tmp/$tools_asset" || err "download failed: $base/$tools_asset (checksums.txt lists it — this release is broken, not merely old)"
    tools_got="$(sha256 "$tmp/$tools_asset")"
    [ "$tools_got" = "$tools_want" ] || err "SHA-256 mismatch for $tools_asset: expected $tools_want, got $tools_got"
    # The bundle is FLAT and regular-files-only by contract
    # (scripts/package-release.sh). It is already SHA-256-verified against this
    # release's own manifest, so these checks are defense in depth — but a SYMLINK
    # member would be dereferenced by the chmod below and could touch a file
    # outside the install dir, so the member TYPE is checked, not just the name.
    # `set -f` disables pathname expansion while the raw names are inspected, so
    # the filter sees exactly what tar will extract.
    # ⚠ `2>&1`: BSD tar writes the -tv listing to STDERR (GNU tar to stdout), so
    # reading stdout alone inspects an EMPTY stream on macOS and reports every
    # archive clean — the filter would die at its own zero. Merging both streams
    # is also fail-closed: any tar diagnostic line fails the leading-`-` test.
    set -f
    if tar -tvzf "$tmp/$tools_asset" 2>&1 | grep -v '^$' | grep -qv '^-'; then
        set +f
        err "$tools_asset contains a non-regular-file member (symlink/dir/device) — refusing"
    fi
    for m in $(tar -tzf "$tmp/$tools_asset"); do
        case "$m" in
            */* | .* | -*) set +f; err "unexpected member name in $tools_asset: $m" ;;
        esac
    done
    tar -xzf "$tmp/$tools_asset" -C "$KX_INSTALL_DIR"
    installed_tools=""
    for m in $(tar -tzf "$tmp/$tools_asset"); do
        chmod +x "$KX_INSTALL_DIR/$m"
        installed_tools="$installed_tools $m"
    done
    set +f
    say " ✓ installed the kx-tools bundle beside kx:$installed_tools"
fi

# --- 7. PATH hint + next steps ----------------------------------------------
case ":${PATH}:" in
    *":$KX_INSTALL_DIR:"*) ;;
    *) say "" ; say "Add to PATH:  export PATH=\"$KX_INSTALL_DIR:\$PATH\"" ;;
esac
say ""
say "Next:  kx serve --dev-allow-local"
say "       # -> web console http://127.0.0.1:8888 (needs a running Ollama daemon"
say "       #    with a pulled model for chat + agents; https://ollama.com)"
say "       KX_SERVE_FS_ROOT=/dir/to/read kx serve --dev-allow-local"
say "       # -> also enables the read-only host fs tools the README's headline"
say "       #    'kx chat --tools' example uses"
# Forward seam: local LLM inference is a separate opt-in (needs a C++ toolchain);
# GPU is cloud-side (Metal works on an Apple host).
if command -v nvidia-smi >/dev/null 2>&1; then
    say ""
    say "(NVIDIA GPU detected — local inference is an opt-in toolchain build: see 'just setup-inference'."
    say " GPU-accelerated inference is cloud-side; on an Apple host, Metal works locally.)"
fi
