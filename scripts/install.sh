#!/bin/sh
# kortecx `kx` installer — the FFI-free runtime, no toolchain required.
#
#   curl -fsSL https://raw.githubusercontent.com/Kortecx/kortecx/main/scripts/install.sh | sh
#
# Detects your platform, downloads a SHA-256-verified prebuilt `kx` from the pinned
# GitHub Release, and installs it to ${KX_INSTALL_DIR:-$HOME/.local/bin}. No sudo,
# no C++ toolchain, idempotent, fail-closed. POSIX sh.
#
# Env:
#   KX_VERSION       release tag to install (default: latest)
#   KX_INSTALL_DIR   install dir (default: $HOME/.local/bin)
set -eu

REPO="Kortecx/kortecx"
KX_VERSION="${KX_VERSION:-latest}"
KX_INSTALL_DIR="${KX_INSTALL_DIR:-$HOME/.local/bin}"

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
asset="kx-$triple"
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

# --- 6. PATH hint + next steps ----------------------------------------------
case ":${PATH}:" in
    *":$KX_INSTALL_DIR:"*) ;;
    *) say "" ; say "Add to PATH:  export PATH=\"$KX_INSTALL_DIR:\$PATH\"" ;;
esac
say ""
say "Next:  kx --help"
say "       kx run --journal /tmp/kx.db --content /tmp/kx-content   # -> a6b5c679... (8/8 committed)"
# Forward seam: local LLM inference is a separate opt-in (needs a C++ toolchain);
# GPU is cloud-side (Metal works on an Apple host).
if command -v nvidia-smi >/dev/null 2>&1; then
    say ""
    say "(NVIDIA GPU detected — local inference is an opt-in toolchain build: see 'just setup-inference'."
    say " GPU-accelerated inference is cloud-side; on an Apple host, Metal works locally.)"
fi
