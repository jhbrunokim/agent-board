#!/usr/bin/env bash
set -euo pipefail
prefix="${AGENT_BOARD_PREFIX:-$HOME/.local}"
binary=""
release=""
download_dir=""
temporary=""
repo="jhbrunokim/agent-board"

cleanup() {
  if [[ -n "$temporary" ]]; then rm -f -- "$temporary"; fi
  if [[ -n "$download_dir" ]]; then rm -rf -- "$download_dir"; fi
}
trap cleanup EXIT

while (($#)); do
  case "$1" in
    --prefix) prefix="${2:?--prefix requires a directory}"; shift 2 ;;
    --binary) binary="${2:?--binary requires an executable}"; shift 2 ;;
    --release) release="${2:?--release requires a tag or latest}"; shift 2 ;;
    --help|-h)
      echo 'Usage: install.sh [--prefix DIR] [--binary PREBUILT | --release TAG_OR_LATEST]'
      echo 'Default: build with Cargo and install under ~/.local/bin'
      exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

case "$prefix" in /*) ;; *) echo 'Prefix must be an absolute path' >&2; exit 2;; esac
if [[ -n "$binary" && -n "$release" ]]; then
  echo 'Choose either --binary or --release' >&2
  exit 2
fi

if [[ -n "$release" ]]; then
  command -v gh >/dev/null || { echo 'GitHub CLI is required for private release downloads' >&2; exit 1; }
  case "$(uname -s):$(uname -m)" in
    Linux:x86_64) asset=agent-board-linux-x86_64 ;;
    Linux:aarch64|Linux:arm64) asset=agent-board-linux-aarch64 ;;
    Darwin:x86_64) asset=agent-board-darwin-x86_64 ;;
    Darwin:arm64) asset=agent-board-darwin-aarch64 ;;
    *) echo 'No release build for this platform. Use a source checkout.' >&2; exit 1 ;;
  esac
  if [[ "$release" == latest ]]; then
    release="$(gh release view --repo "$repo" --json tagName --jq .tagName)"
  fi
  if [[ ! "$release" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo 'Expected a release tag in vMAJOR.MINOR.PATCH format' >&2
    exit 2
  fi
  download_dir="$(mktemp -d)"
  gh release download "$release" --repo "$repo" --dir "$download_dir" \
    --pattern "$asset" --pattern SHA256SUMS
  binary="$download_dir/$asset"
  expected="$(awk -v asset="$asset" '$2 == asset && NF == 2 {print $1}' "$download_dir/SHA256SUMS")"
  if [[ ! "$expected" =~ ^[0-9a-f]{64}$ ]]; then
    echo 'Release checksum is missing or ambiguous' >&2
    exit 1
  fi
  if command -v sha256sum >/dev/null; then
    actual="$(sha256sum < "$binary")"
  elif command -v shasum >/dev/null; then
    actual="$(shasum -a 256 < "$binary")"
  else
    echo 'Install sha256sum or shasum to verify the release' >&2
    exit 1
  fi
  if [[ "${actual%% *}" != "$expected" ]]; then
    echo 'Release checksum mismatch' >&2
    exit 1
  fi
  chmod 755 "$binary"
  if [[ "$("$binary" --version)" != "agent-board ${release#v}" ]]; then
    echo 'Binary version does not match the release tag' >&2
    exit 1
  fi
elif [[ -z "$binary" ]]; then
  command -v cargo >/dev/null || { echo 'Rust is required for source install. Install Rust using rustup, then retry.' >&2; exit 1; }
  source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
  cargo build --release --locked --manifest-path "$source_dir/Cargo.toml" --target-dir "$source_dir/target"
  binary="$source_dir/target/release/agent-board"
fi

[[ -f "$binary" && -x "$binary" ]] || { echo 'Binary must be an executable regular file' >&2; exit 1; }
"$binary" --help >/dev/null
mkdir -p -- "$prefix/bin"
destination="$prefix/bin/agent-board"
if [[ -L "$destination" || ( -e "$destination" && ! -f "$destination" ) ]]; then
  echo 'Install destination must be a regular file' >&2
  exit 1
fi
temporary="$(mktemp "$prefix/bin/.agent-board.XXXXXXXX")"
install -m 755 -- "$binary" "$temporary"
mv -f -- "$temporary" "$destination"
printf 'Installed: %s\n' "$destination"
printf 'Next: %s --help\n' "$destination"
printf 'Add %s/bin to PATH if needed.\n' "$prefix"
