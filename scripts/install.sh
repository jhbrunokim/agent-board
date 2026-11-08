#!/usr/bin/env bash
set -euo pipefail
prefix="${AGENT_BOARD_PREFIX:-$HOME/.local}"
binary=""
while (($#)); do
  case "$1" in
    --prefix) prefix="${2:?--prefix requires a directory}"; shift 2 ;;
    --binary) binary="${2:?--binary requires an executable}"; shift 2 ;;
    --help|-h) echo 'Usage: install.sh [--prefix DIR] [--binary PREBUILT]'; echo 'Default: build release with Cargo and install under ~/.local/bin'; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$prefix" in /*) ;; *) echo 'Prefix must be an absolute path' >&2; exit 2;; esac
if [[ -z "$binary" ]]; then
  command -v cargo >/dev/null || { echo 'Rust is required for source install. Install Rust using rustup, then retry.' >&2; exit 1; }
  source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
  cargo build --release --locked --manifest-path "$source_dir/Cargo.toml"
  binary="$source_dir/target/release/agent-board"
fi
[[ -f "$binary" && -x "$binary" ]] || { echo 'Binary must be an executable regular file' >&2; exit 1; }
"$binary" --help >/dev/null
mkdir -p -- "$prefix/bin"
destination="$prefix/bin/agent-board"
[[ ! -L "$destination" ]] || { echo 'Refusing to replace symlink destination' >&2; exit 1; }
temporary="$(mktemp "$prefix/bin/.agent-board.XXXXXXXX")"
trap 'rm -f -- "$temporary"' EXIT
install -m 755 -- "$binary" "$temporary"
mv -f -- "$temporary" "$destination"
printf 'Installed: %s\n' "$destination"
printf 'Next: %s --help\n' "$destination"
printf 'Add %s/bin to PATH if needed. Database and tokens have not been created.\n' "$prefix"
