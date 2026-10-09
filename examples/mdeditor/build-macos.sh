#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/../.."
if [[ "$(uname -s)" != Darwin ]]; then
    echo "此打包脚本仅支持 macOS。" >&2
    exit 1
fi

cargo build --example mdeditor
bundle="target/Glint Markdown.app"
resources="$bundle/Contents/Resources"
mkdir -p "$bundle/Contents/MacOS" "$resources"
cp target/debug/examples/mdeditor "$bundle/Contents/MacOS/mdeditor"
cp examples/mdeditor/assets/Info.plist "$bundle/Contents/Info.plist"

iconset="$(mktemp -d)/mdeditor.iconset"
mkdir -p "$iconset"
trap 'rm -rf "${iconset%/*}"' EXIT
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" examples/mdeditor/assets/mdeditor-icon.png \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" examples/mdeditor/assets/mdeditor-icon.png \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/mdeditor.icns"
echo "已生成：$PWD/$bundle"
