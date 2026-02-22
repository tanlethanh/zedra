#!/bin/sh
# Zedra Host installer
# Usage: curl -fsSL https://raw.githubusercontent.com/tanlethanh/zedra/main/scripts/install.sh | sh
#        curl -fsSL ... | sh -s -- --version v0.1.0 --prefix /usr/local/bin
set -eu

REPO="tanlethanh/zedra"
BINARY="zedra"
DEFAULT_PREFIX="${HOME}/.local/bin"

# --- Argument parsing ---

VERSION=""
PREFIX=""

while [ $# -gt 0 ]; do
    case "$1" in
        --version)  VERSION="$2"; shift 2 ;;
        --prefix)   PREFIX="$2"; shift 2 ;;
        --help|-h)
            echo "Usage: install.sh [--version VERSION] [--prefix DIR]"
            echo ""
            echo "Options:"
            echo "  --version VERSION  Install a specific version (e.g. v0.1.0)"
            echo "  --prefix DIR       Install directory (default: ~/.local/bin)"
            echo "                     Can also be set via ZEDRA_PREFIX env var"
            exit 0
            ;;
        *)  echo "Unknown option: $1"; exit 1 ;;
    esac
done

PREFIX="${PREFIX:-${ZEDRA_PREFIX:-${DEFAULT_PREFIX}}}"

# --- Platform detection ---

detect_platform() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin) os="apple-darwin" ;;
        Linux)  os="unknown-linux-gnu" ;;
        *)      echo "Error: unsupported OS: $os"; exit 1 ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        arm64|aarch64) arch="aarch64" ;;
        *)             echo "Error: unsupported architecture: $arch"; exit 1 ;;
    esac

    echo "${arch}-${os}"
}

# --- Version resolution ---

resolve_version() {
    if [ -n "$VERSION" ]; then
        echo "$VERSION"
        return
    fi

    # Follow the /releases/latest redirect to get the tag name.
    # This avoids the GitHub API (no rate limits, no jq needed).
    url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest" 2>/dev/null)" || {
        echo "Error: failed to resolve latest version. Specify one with --version." >&2
        exit 1
    }

    # url looks like https://github.com/tanlethanh/zedra/releases/tag/v0.1.0
    tag="${url##*/}"
    if [ -z "$tag" ] || [ "$tag" = "releases" ]; then
        echo "Error: could not determine latest release tag." >&2
        exit 1
    fi
    echo "$tag"
}

# --- Checksum verification (best-effort) ---

verify_checksum() {
    archive="$1"
    checksum_url="$2"

    # Try to download the .sha256 file
    expected="$(curl -fsSL "$checksum_url" 2>/dev/null)" || {
        echo "  (checksum file not available, skipping verification)"
        return 0
    }

    expected_hash="$(echo "$expected" | awk '{print $1}')"

    # Find a SHA256 tool
    if command -v sha256sum >/dev/null 2>&1; then
        actual_hash="$(sha256sum "$archive" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        actual_hash="$(shasum -a 256 "$archive" | awk '{print $1}')"
    else
        echo "  (no sha256sum or shasum found, skipping verification)"
        return 0
    fi

    if [ "$actual_hash" != "$expected_hash" ]; then
        echo "Error: checksum mismatch!"
        echo "  expected: $expected_hash"
        echo "  actual:   $actual_hash"
        exit 1
    fi

    echo "  Checksum verified."
}

# --- Main ---

main() {
    platform="$(detect_platform)"
    version="$(resolve_version)"

    echo "Installing ${BINARY} ${version} for ${platform}..."

    base_url="https://github.com/${REPO}/releases/download/${version}"
    archive_name="${BINARY}-${platform}.tar.gz"
    archive_url="${base_url}/${archive_name}"
    checksum_url="${base_url}/${archive_name}.sha256"

    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    echo "  Downloading ${archive_url}..."
    curl -fsSL -o "${tmpdir}/${archive_name}" "$archive_url" || {
        echo "Error: download failed. Check that version '${version}' exists at:"
        echo "  https://github.com/${REPO}/releases"
        exit 1
    }

    verify_checksum "${tmpdir}/${archive_name}" "$checksum_url"

    echo "  Extracting..."
    tar xzf "${tmpdir}/${archive_name}" -C "$tmpdir"

    echo "  Installing to ${PREFIX}..."
    mkdir -p "$PREFIX"
    mv "${tmpdir}/${BINARY}" "${PREFIX}/${BINARY}"
    chmod +x "${PREFIX}/${BINARY}"

    echo ""
    echo "Installed ${BINARY} to ${PREFIX}/${BINARY}"

    # Check if PREFIX is in PATH
    case ":${PATH}:" in
        *":${PREFIX}:"*) ;;
        *)
            echo ""
            echo "WARNING: ${PREFIX} is not in your PATH."
            echo "Add it by running:"
            echo ""
            echo "  echo 'export PATH=\"${PREFIX}:\$PATH\"' >> ~/.profile"
            echo "  source ~/.profile"
            echo ""
            ;;
    esac

    echo "Run '${BINARY} --help' to get started."
}

main
