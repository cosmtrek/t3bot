#!/bin/sh
# Install or upgrade t3bot: the binary, the @t3bot skill and the systemd user timers.
#
#   curl -fsSL https://raw.githubusercontent.com/cosmtrek/t3bot/main/scripts/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version v0.2.0
#   ./scripts/install.sh --from-source          # from a checkout, builds with cargo
#
# Options:
#   --version vX.Y.Z   install this release (default: latest)
#   --from-source      build the checkout this script lives in instead of downloading
#   --force            reinstall even if the version is already installed
#   --no-systemd       do not install or enable the systemd timers
# Env: T3BOT_BIN_DIR (default ~/.local/bin).
#
# A fresh install runs `t3bot init` (asks for the inbox and projects); an upgrade keeps
# ~/.t3bot/config.toml and ~/.t3bot/prompt.md as they are and runs `t3bot doctor`.
set -eu

REPO=cosmtrek/t3bot
BIN_DIR=${T3BOT_BIN_DIR:-$HOME/.local/bin}
version=latest
from_source=0
force=0
systemd=1

die() { echo "t3bot install: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || die "--version needs a value"; version=$2; shift 2 ;;
    --version=*) version=${1#*=}; shift ;;
    --from-source) from_source=1; shift ;;
    --force) force=1; shift ;;
    --no-systemd) systemd=0; shift ;;
    -h|--help) sed -n '2,16p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

installed=""
if [ -x "$BIN_DIR/t3bot" ]; then
  installed="v$("$BIN_DIR/t3bot" --version 2>/dev/null | awk '{print $2}')"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$BIN_DIR"

if [ "$from_source" = 1 ]; then
  src=$(cd "$(dirname "$0")/.." && pwd)
  [ -f "$src/Cargo.toml" ] || die "--from-source must be run from a t3bot checkout"
  command -v cargo >/dev/null || die "cargo not found"
  (cd "$src" && cargo build --release --locked --quiet)
  bin="$src/target/release/t3bot"
  assets=$src
else
  [ "$(uname -s)" = Linux ] || die "prebuilt binaries are Linux only; use --from-source"
  case "$(uname -m)" in
    x86_64|amd64) target=x86_64-unknown-linux-musl ;;
    aarch64|arm64) target=aarch64-unknown-linux-musl ;;
    *) die "no prebuilt binary for $(uname -m); use --from-source" ;;
  esac
  command -v curl >/dev/null || die "curl not found"

  if [ "$version" = latest ]; then
    url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") \
      || die "cannot reach GitHub"
    version=${url##*/}
    case "$version" in v*) ;; *) die "no release found" ;; esac
  fi
  case "$version" in v*) ;; *) version="v$version" ;; esac

  if [ "$installed" = "$version" ] && [ "$force" = 0 ]; then
    echo "t3bot $version is already installed in $BIN_DIR (use --force to reinstall)"
    exit 0
  fi

  name="t3bot-$version-$target"
  base="https://github.com/$REPO/releases/download/$version"
  echo "downloading $name ..."
  curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz" || die "download failed: $base/$name.tar.gz"
  curl -fsSL -o "$tmp/$name.tar.gz.sha256" "$base/$name.tar.gz.sha256" || die "checksum download failed"
  (cd "$tmp" && sha256sum -c --quiet "$name.tar.gz.sha256") || die "checksum mismatch"
  tar xzf "$tmp/$name.tar.gz" -C "$tmp"
  bin="$tmp/$name/t3bot"
  assets="$tmp/$name"
fi

# Replace atomically so a running tick never sees a half-written binary.
install -m 755 "$bin" "$BIN_DIR/.t3bot.new"
mv -f "$BIN_DIR/.t3bot.new" "$BIN_DIR/t3bot"
new="v$("$BIN_DIR/t3bot" --version | awk '{print $2}')"
if [ -n "$installed" ] && [ "$installed" != "$new" ]; then
  echo "t3bot $installed → $new ($BIN_DIR/t3bot)"
else
  echo "t3bot $new → $BIN_DIR/t3bot"
fi

# Global skill locations read by the providers T3 runs (Claude, Codex, Antigravity).
for dir in "$HOME/.claude/skills" "$HOME/.agents/skills" "$HOME/.gemini/antigravity-cli/skills"; do
  if [ -d "$(dirname "$dir")" ]; then
    mkdir -p "$dir/t3bot"
    cp "$assets/skill/t3bot/SKILL.md" "$dir/t3bot/SKILL.md"
    echo "skill → $dir/t3bot"
  fi
done

if [ "$systemd" = 1 ]; then
  if command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; then
    unit_dir="$HOME/.config/systemd/user"
    mkdir -p "$unit_dir"
    cp "$assets"/contrib/systemd/* "$unit_dir/"
    systemctl --user daemon-reload
    systemctl --user enable --now t3bot-morning.timer t3bot-tick.timer
    systemctl --user list-timers 't3bot-*' --no-pager
  else
    echo "no systemd user session; skipped timers (run \`t3bot tick\` from your own scheduler)"
  fi
fi

# Config last, so doctor sees the timers. A fresh install asks for the values when a
# terminal is available (also under `curl | sh`); an upgrade never touches the config.
echo
cfg="${T3BOT_HOME:-$HOME/.t3bot}/config.toml"
if [ -f "$cfg" ]; then
  "$BIN_DIR/t3bot" doctor || echo "(fix the ✗ items above; \`t3bot doctor\` checks again)"
elif ( : </dev/tty ) 2>/dev/null; then
  "$BIN_DIR/t3bot" init </dev/tty || true
else
  "$BIN_DIR/t3bot" init --no-input || true
  echo "next: t3bot init --inbox <path> --project <path> ...   (or run \`t3bot init\` in a terminal)"
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "note: $BIN_DIR is not in PATH" ;;
esac
