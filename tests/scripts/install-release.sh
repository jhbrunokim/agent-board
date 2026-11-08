#!/usr/bin/env bash
set -euo pipefail
binary="$(cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/bin" "$scratch/assets"
export TEST_ASSETS="$scratch/assets"
export TEST_TAG="v$("$binary" --version | awk '{print $2}')"
case "$(uname -s):$(uname -m)" in
  Linux:x86_64) asset=agent-board-linux-x86_64 ;;
  Linux:aarch64|Linux:arm64) asset=agent-board-linux-aarch64 ;;
  Darwin:x86_64) asset=agent-board-darwin-x86_64 ;;
  Darwin:arm64) asset=agent-board-darwin-aarch64 ;;
  *) exit 1 ;;
esac
cp "$binary" "$TEST_ASSETS/$asset"
(
  cd "$TEST_ASSETS"
  if command -v sha256sum >/dev/null; then
    sha256sum "$asset" > SHA256SUMS
  else
    shasum -a 256 "$asset" > SHA256SUMS
  fi
)
cat > "$scratch/bin/gh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == release ]]
shift
if [[ "$1" == view ]]; then
  printf '%s\n' "$TEST_TAG"
  exit
fi
[[ "$1" == download && "$2" == "$TEST_TAG" ]]
shift 2
[[ "$1" == --repo && "$2" == jhbrunokim/agent-board && "$3" == --dir ]]
target="$4"
shift 4
while (($#)); do
  [[ "$1" == --pattern ]]
  cp "$TEST_ASSETS/$2" "$target/$2"
  shift 2
done
MOCK
chmod +x "$scratch/bin/gh"
export PATH="$scratch/bin:$PATH"
prefix="$scratch/install location"
scripts/install.sh --release latest --prefix "$prefix"
cmp "$binary" "$prefix/bin/agent-board"
printf 'corruption' >> "$TEST_ASSETS/$asset"
if scripts/install.sh --release "$TEST_TAG" --prefix "$prefix" 2>"$scratch/error"; then
  echo 'Corrupted release was accepted' >&2
  exit 1
fi
grep -q 'checksum mismatch' "$scratch/error"
cmp "$binary" "$prefix/bin/agent-board"
