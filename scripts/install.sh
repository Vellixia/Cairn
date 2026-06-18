#!/bin/sh
# Cairn installer (Linux/macOS).
#
#   curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh -s -- pair CAIRN-XXXX
#
# Honors: CAIRN_REPO, CAIRN_INSTALL_DIR, CAIRN_VERSION, CAIRN_INSTALL_SKIP_VERIFY.
set -eu

REPO="${CAIRN_REPO:-Vellixia/Cairn}"
BIN="cairn"
CLI_BIN="cairn-cli"
INSTALL_DIR="${CAIRN_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${CAIRN_VERSION:-latest}"
BASE_URL="https://github.com/$REPO/releases"

say()  { printf '\033[36m›\033[0m %s\n' "$1"; }
warn() { printf '\033[33m⚠ %s\033[0m\n' "$1" >&2; }
err()  { printf '\033[31m✗ %s\033[0m\n' "$1" >&2; exit 1; }

# Compute SHA-256 of a file, writing "<hex>  <filename>" to stdout. Prefers GNU sha256sum
# (Linux); falls back to perl-based shasum (macOS). Aborts if neither is available.
sha256_file() {
    file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file"
    elif command -v shasum >/dev/null 2>&1; then
        # shasum -a 256 prints "<hex>  <filename>" in the same shape as sha256sum.
        shasum -a 256 "$file"
    else
        err "neither sha256sum nor shasum is installed; cannot verify release artifact"
    fi
}

# Extract the expected SHA-256 for `archive_name` from a SHA256SUMS file. The file is in the
# standard "sha256sum" format: "<hex>  <filename>", one per line. Returns 0 on match, 1 on
# miss. Echoes the expected hex digest on stdout when found.
expected_hash() {
    sums_file="$1"
    archive_name="$2"
    # awk: match a line whose 2nd field (or " *" path) ends with the archive name. Compare
    # basename-only — `sha256sum` may prefix with "./" or a relative path.
    awk -v want="$archive_name" '
        {
            # Strip CR for Windows-generated files; $1 is the hex, the rest is the filename.
            sub(/\r$/, "")
            hex = $1
            # Reconstruct the filename (may contain spaces): drop the first field + whitespace.
            fname = $0
            sub(/^[[:space:]]*[0-9a-fA-F]*[[:space:]]*/, "", fname)
            # Strip leading "./" and any path prefix; compare base name.
            n = split(fname, parts, "/")
            base = parts[n]
            if (base == want) { print hex; found = 1; exit 0 }
        }
        END { if (!found) exit 1 }
    ' "$sums_file"
}

detect_target() {
    os="$(uname -s)"; arch="$(uname -m)"
    case "$os" in
        Linux)  os="unknown-linux-gnu" ;;
        Darwin) os="apple-darwin" ;;
        *) err "unsupported OS: $os (use install.ps1 on Windows)" ;;
    esac
    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        arm64|aarch64) arch="aarch64" ;;
        *) err "unsupported arch: $arch" ;;
    esac
    printf '%s-%s' "$arch" "$os"
}

# Resolve "latest" to the concrete tag name via the GitHub releases/latest redirect. If
# $CAIRN_VERSION is set, use it verbatim.
resolve_version() {
    if [ "$VERSION" != "latest" ]; then
        printf '%s' "$VERSION"
        return 0
    fi
    # HEAD the release-listing endpoint. -sI prints headers only; the Location header (or
    # the redirected URL) carries the versioned tag. We use -o /dev/null -w to grab the
    # final URL after redirects.
    final="$(curl -fsSL -o /dev/null -w '%{url_effective}' "$BASE_URL/latest" || true)"
    if [ -z "$final" ]; then
        err "could not resolve latest release for $REPO (network error?)"
    fi
    # Final URL looks like: https://github.com/<repo>/releases/tag/<tag>
    case "$final" in
        */releases/tag/*) printf '%s' "${final##*/releases/tag/}" ;;
        *) err "unexpected redirect URL when resolving latest: $final" ;;
    esac
}

install_binary() {
    target="$(detect_target)"
    tag="$(resolve_version)"
    archive_name="cairn-$target.tar.gz"
    archive_url="$BASE_URL/download/$tag/$archive_name"
    sums_url="$BASE_URL/download/$tag/SHA256SUMS"

    say "Installing cairn $tag ($target) -> $INSTALL_DIR"
    mkdir -p "$INSTALL_DIR"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT INT TERM

    if ! curl -fsSL "$archive_url" -o "$tmp/$archive_name" 2>/dev/null; then
        if command -v cargo >/dev/null 2>&1; then
            warn "No prebuilt release found; building from source with cargo…"
            cargo install --git "https://github.com/$REPO" cairn-server cairn-cli
            return 0
        fi
        err "no prebuilt binary available for $target and cargo is not installed"
    fi

    # Verify the archive before unpacking — defence against a compromised or partial download.
    if [ "${CAIRN_INSTALL_SKIP_VERIFY:-}" = "1" ]; then
        warn "================================================================"
        warn "  !!! CAIRN_INSTALL_SKIP_VERIFY=1 set — checksum verification !!!"
        warn "  !!! SKIPPED. You are about to execute an UNVERIFIED binary.  !!!"
        warn "  !!! This is a SECURITY RISK. Only use for local debugging.    !!!"
        warn "================================================================"
    else
        say "Verifying SHA-256 checksum…"
        if ! curl -fsSL "$sums_url" -o "$tmp/SHA256SUMS" 2>/dev/null; then
            err "could not download SHA256SUMS from $sums_url — refusing to install unverified artifact. Re-run after a few seconds (the release job may still be finalizing) or pin CAIRN_VERSION to a known-good release."
        fi
        if ! expected="$(expected_hash "$tmp/SHA256SUMS" "$archive_name")"; then
            err "$archive_name not listed in SHA256SUMS — refusing to install unverified artifact."
        fi
        if ! actual="$(sha256_file "$tmp/$archive_name" | awk '{print $1}')"; then
            err "could not hash downloaded archive"
        fi
        if [ "$expected" != "$actual" ]; then
            err "checksum mismatch for $archive_name: expected $expected, got $actual"
        fi
        say "Checksum OK ($actual)"
    fi

    tar -xzf "$tmp/$archive_name" -C "$tmp"
    mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"
    mv "$tmp/$CLI_BIN" "$INSTALL_DIR/$CLI_BIN"
    chmod +x "$INSTALL_DIR/$BIN" "$INSTALL_DIR/$CLI_BIN"
}

install_binary
case ":$PATH:" in
    *":$INSTALL_DIR:"*) : ;;
    *) say "Add $INSTALL_DIR to your PATH to use \`cairn\` everywhere." ;;
esac

# Optional: `... | sh -s -- pair CODE SERVER` pairs the device, then wires up local agents.
if [ "${1:-}" = "pair" ] && [ -n "${2:-}" ] && [ -n "${3:-}" ]; then
    say "Pairing this device…"
    "$INSTALL_DIR/$CLI_BIN" pair "$2" --server "$3" || err "pairing failed"
    say "Configuring installed agents…"
    "$INSTALL_DIR/$CLI_BIN" setup --all --server "$3" || true
fi

say "Done. Start the server with:  cairn serve"
say "Configure agents with:        cairn-cli setup <agent> --server <url>"
