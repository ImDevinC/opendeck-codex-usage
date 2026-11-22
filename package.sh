#!/bin/sh
set -eu
cd "$(dirname "$0")"

mode=${1:-package}
case "$mode" in
	stage|package) ;;
	*) echo "Usage: sh package.sh [stage|package]" >&2; exit 2 ;;
esac

target=$(rustc -vV | sed -n 's/^host: //p')
case "$target" in
	x86_64-pc-windows-msvc) extension=.exe ;;
	x86_64-apple-darwin|aarch64-apple-darwin|x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) extension= ;;
	*) echo "Unsupported build target: $target" >&2; exit 1 ;;
esac

bundle=com.imdevinc.codexusage.sdPlugin
archive=com.imdevinc.codexusage.zip
cargo build --locked --release --target "$target"
mkdir -p "$bundle"
cp assets/manifest.json assets/pi.html assets/icon.svg "$bundle/"
cp "${CARGO_TARGET_DIR:-target}/$target/release/oacodex-usage$extension" "$bundle/oacodex-usage-$target$extension"
if [ "$mode" = package ]; then
	# Rebuild the archive so it does not retain files from a prior build.
	rm -f "$archive"
	zip -q -r "$archive" "$bundle"
fi
