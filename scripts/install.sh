#!/bin/sh
# Cairn installer (Linux/macOS).
#
#   curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh
#
# Cairn runs inside Docker. This script downloads the docker-compose.yml
# and a template .env into ~/.config/cairn/ and prints the next step.
#
# Honors: CAIRN_INSTALL_DIR (defaults to ~/.config/cairn).
set -eu

REPO="${CAIRN_REPO:-Vellixia/Cairn}"
INSTALL_DIR="${CAIRN_INSTALL_DIR:-$HOME/.config/cairn}"
RAW_BASE="https://raw.githubusercontent.com/$REPO/main"

say()  { printf '\033[36m>\033[0m %s\n' "$1"; }
warn() { printf '\033[33m!\033[0m %s\n' "$1" >&2; }
err()  { printf '\033[31mX\033[0m %s\n' "$1" >&2; exit 1; }

require() {
    for tool in "$@"; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            err "missing required tool: $tool"
        fi
    done
}

require docker curl

say "Cairn install — Docker-only setup"
say "Target directory: $INSTALL_DIR"

mkdir -p "$INSTALL_DIR"

curl -fsSL "$RAW_BASE/docker-compose.yml" -o "$INSTALL_DIR/docker-compose.yml" \
    || err "could not download docker-compose.yml from $RAW_BASE (network error?)"
say "wrote $INSTALL_DIR/docker-compose.yml"

curl -fsSL "$RAW_BASE/.env.example" -o "$INSTALL_DIR/.env.example" \
    || err "could not download .env.example from $RAW_BASE (network error?)"
say "wrote $INSTALL_DIR/.env.example"

if [ ! -f "$INSTALL_DIR/.env" ]; then
    cp "$INSTALL_DIR/.env.example" "$INSTALL_DIR/.env"
    say "created $INSTALL_DIR/.env from template"
    say ""
    say "Next steps:"
    say "  1. Edit $INSTALL_DIR/.env and set:"
    say "       CAIRN_ADMIN_USERNAME=admin"
    say "       CAIRN_ADMIN_PASSWORD=<a strong password, 8+ chars>"
    say "       MINIO_ROOT_USER=<random>"
    say "       MINIO_ROOT_PASSWORD=<random>"
    say "  2. cd $INSTALL_DIR"
    say "  3. docker compose up -d"
    say "  4. Open http://127.0.0.1:7777 and log in"
else
    say "$INSTALL_DIR/.env already exists — leaving it alone"
    say ""
    say "Next step: cd $INSTALL_DIR && docker compose up -d"
fi
