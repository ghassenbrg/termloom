#!/bin/sh
# TermLoom installer.
#
#   curl -fsSL https://termloom.ghassen.io/install.sh | sh
#
# Downloads the release binary for this platform, verifies its checksum and
# installs it. Nothing is built, nothing runs as root, and the script prints
# every path it touches.
#
# Environment:
#   TERMLOOM_VERSION       version to install (default: the latest release)
#   TERMLOOM_INSTALL_DIR   where to put the binary (default: ~/.local/bin)
#   TERMLOOM_BASE_URL      override the download host (mirrors, testing)

set -eu

REPO="ghassenbrg/termloom"
BASE_URL="${TERMLOOM_BASE_URL:-https://github.com/${REPO}/releases/download}"
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
INSTALL_DIR="${TERMLOOM_INSTALL_DIR:-${HOME}/.local/bin}"

say() { printf '%s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || err "this installer needs $1"; }

usage() {
    cat <<'USAGE'
Install TermLoom, an agent-native development environment for your terminal.

Usage: install.sh [--version <version>] [--dir <directory>] [--help]

  --version   install a specific release (e.g. 0.1.0); default is the latest
  --dir       install into this directory; default is ~/.local/bin

The same settings can be given as TERMLOOM_VERSION and TERMLOOM_INSTALL_DIR.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || err "--version needs a value"; TERMLOOM_VERSION="$2"; shift 2 ;;
        --dir)     [ $# -ge 2 ] || err "--dir needs a value"; INSTALL_DIR="$2"; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *)         err "unknown option: $1 (try --help)" ;;
    esac
done

# ── platform ──────────────────────────────────────────────────────────────
os="$(uname -s)"
arch="$(uname -m)"
case "${os}:${arch}" in
    Darwin:arm64)          target="aarch64-apple-darwin" ;;
    Darwin:x86_64)         target="x86_64-apple-darwin" ;;
    Linux:x86_64)          target="x86_64-unknown-linux-gnu" ;;
    Linux:aarch64|Linux:arm64)
        err "linux ${arch} has no published build yet; build from source with: cargo install --git https://github.com/${REPO}" ;;
    *)
        err "unsupported platform ${os} ${arch}; TermLoom publishes macOS and Linux x86_64 builds" ;;
esac

# ── tools ─────────────────────────────────────────────────────────────────
need tar
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
    fetch_stdout() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
    fetch_stdout() { wget -qO- "$1"; }
else
    err "this installer needs curl or wget"
fi

if command -v sha256sum >/dev/null 2>&1; then
    checksum() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    checksum() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    checksum() { echo ""; }
fi

# ── version ───────────────────────────────────────────────────────────────
version="${TERMLOOM_VERSION:-}"
if [ -z "$version" ]; then
    say "Looking up the latest TermLoom release..."
    version="$(fetch_stdout "$API_URL" 2>/dev/null \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' \
        | head -n 1 || true)"
    [ -n "$version" ] || err "could not determine the latest release; pass --version, or check https://github.com/${REPO}/releases"
fi
version="${version#v}"

archive="termloom-${target}.tar.gz"
url="${BASE_URL}/v${version}/${archive}"
sums_url="${BASE_URL}/v${version}/SHA256SUMS"

# ── download ──────────────────────────────────────────────────────────────
tmp="$(mktemp -d 2>/dev/null || mktemp -d -t termloom)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "Downloading TermLoom ${version} for ${target}..."
fetch "$url" "${tmp}/${archive}" || err "download failed: ${url}"

# ── verify ────────────────────────────────────────────────────────────────
if fetch "$sums_url" "${tmp}/SHA256SUMS" 2>/dev/null; then
    expected="$(sed -n "s/^\([0-9a-f]\{64\}\)[[:space:]]*[*]\{0,1\}${archive}$/\1/p" "${tmp}/SHA256SUMS" | head -n 1)"
    actual="$(checksum "${tmp}/${archive}")"
    if [ -z "$actual" ]; then
        say "warning: no sha256 tool found, skipping checksum verification"
    elif [ -z "$expected" ]; then
        say "warning: ${archive} is not listed in SHA256SUMS, skipping verification"
    elif [ "$expected" != "$actual" ]; then
        err "checksum mismatch for ${archive}: expected ${expected}, got ${actual}"
    else
        say "Checksum verified."
    fi
else
    say "warning: no SHA256SUMS published for ${version}, skipping verification"
fi

# ── install ───────────────────────────────────────────────────────────────
tar -xzf "${tmp}/${archive}" -C "$tmp"
binary="$(find "$tmp" -type f -name termloom -perm -u+x 2>/dev/null | head -n 1)"
[ -n "$binary" ] || err "the archive did not contain a termloom binary"

mkdir -p "$INSTALL_DIR" || err "cannot create ${INSTALL_DIR}"
# Replace atomically so a running TermLoom keeps its open binary.
install_path="${INSTALL_DIR}/termloom"
cp "$binary" "${install_path}.new" || err "cannot write to ${INSTALL_DIR}"
chmod +x "${install_path}.new"
mv "${install_path}.new" "$install_path"

say "Installed ${install_path}"

installed_version="$("$install_path" --version 2>/dev/null || true)"
[ -n "$installed_version" ] || err "the installed binary did not run; your platform may be incompatible"
say "$installed_version"

# ── PATH advice ───────────────────────────────────────────────────────────
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        say ""
        say "${INSTALL_DIR} is not on your PATH. Add it:"
        say ""
        case "${SHELL:-}" in
            */zsh)  say "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.zshrc && exec zsh" ;;
            */fish) say "  fish_add_path ${INSTALL_DIR}" ;;
            *)      say "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.profile" ;;
        esac
        ;;
esac

say ""
say "Run 'termloom doctor' to check this machine, then 'termloom .' in a repository."
