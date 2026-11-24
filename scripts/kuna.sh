#!/usr/bin/env bash
# kuna.sh — fetch the pinned kuna into target/kuna/, verified against the sha256 pinned in ./kuna-version.
# Prints the binary's path. Needs curl and tar.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(head -n1 kuna-version | tr -d '[:space:]')
[[ $version =~ ^[0-9]+(\.[0-9]+)+$ ]] || { echo "kuna.sh: bad version '$version' in kuna-version" >&2; exit 1; }
dir=target/kuna
bin=$dir/kuna
untar=tar
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) platform=macos-arm64 ext=tar.gz ;;
  Darwin-x86_64) platform=macos-x86_64 ext=tar.gz ;;
  Linux-x86_64) platform=linux-x86_64 ext=tar.gz ;;
  Linux-aarch64) platform=linux-arm64 ext=tar.gz ;;
  MINGW* | MSYS* | CYGWIN*)
    platform=windows-x86_64 ext=zip bin=$dir/kuna.exe
    untar="$(cygpath -u "$SYSTEMROOT")/System32/tar.exe" ;;
  *) echo "kuna.sh: no kuna release for $(uname -sm)" >&2; exit 1 ;;
esac
pinned=$(awk -v p="$platform" '$1 == p { print $2 }' kuna-version)
[[ $pinned =~ ^[0-9a-f]{64}$ ]] || { echo "kuna.sh: no sha256 pinned for $platform in kuna-version" >&2; exit 1; }

if [[ -z ${CI:-} && -x $bin && $("$bin" --version 2>/dev/null) == "kuna $version" ]]; then
  echo "$bin"
  exit 0
fi

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

asset="kuna-v$version-$platform.$ext"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/$asset" "https://github.com/Noelo-Lab/kuna/releases/download/v$version/$asset"
[[ $(sha256 "$tmp/$asset") == "$pinned" ]] || { echo "kuna.sh: $asset does not match the sha256 pinned in kuna-version" >&2; exit 1; }

rm -rf "$dir"
mkdir -p "$dir"
"$untar" -xf "$tmp/$asset" -C "$dir" --strip-components=1
[[ $("$bin" --version) == "kuna $version" ]] || { echo "kuna.sh: $bin is not kuna $version" >&2; exit 1; }
echo "$bin"
