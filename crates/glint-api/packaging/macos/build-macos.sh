#!/bin/bash
# Build a native release .app and a drag-to-Applications DMG on macOS.
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
    echo "Build this installer on macOS." >&2
    exit 1
fi
repo="$(cd "$(dirname "$0")/../../../.." && pwd)"
cd "$repo"
for tool in cargo rustc plutil xcrun sips iconutil codesign hdiutil ditto shasum otool; do
    command -v "$tool" >/dev/null || { echo "Required tool is missing: $tool" >&2; exit 1; }
done
version="$(awk -F '"' '/^version[[:space:]]*=/ { print $2; exit }' crates/glint-api/Cargo.toml)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Use a stable application version." >&2; exit 1; }
host="$(rustc -vV | awk '/^host:/ {print $2}')"
case "$host" in
    aarch64-apple-darwin) arch=arm64 ;;
    x86_64-apple-darwin) arch=x86_64 ;;
    *) echo "Unsupported macOS Rust host: $host" >&2; exit 1 ;;
esac
# An explicit native target also avoids an unrelated build.target in user Cargo config.
cargo build -p glint-api --release --locked --target "$host"
target_dir="$(cargo metadata --no-deps --format-version 1 | plutil -extract target_directory raw -o - -)"
[[ -n "$target_dir" && "$target_dir" != / ]] || { echo "Invalid Cargo target directory." >&2; exit 1; }
work_root="$target_dir/macos-installer"
output="$work_root/output"
mkdir -p "$output"
work="$(mktemp -d "$work_root/.build.XXXXXX")"
mounted=false
cleanup() {
    if [[ "$mounted" == true ]]; then hdiutil detach "$work/mounted" >/dev/null || true; fi
    rm -rf "$work"
}
trap cleanup EXIT
stage="$work/stage"
app="$stage/Glint API.app"
resources="$app/Contents/Resources"
mkdir -p "$app/Contents/MacOS" "$resources"
cp "$target_dir/$host/release/glint-api" "$app/Contents/MacOS/glint-api"
chmod 755 "$app/Contents/MacOS/glint-api"
strip -x "$app/Contents/MacOS/glint-api"
# Intel binaries may use LC_VERSION_MIN_MACOSX; Apple Silicon uses LC_BUILD_VERSION.
minimum_os="$(otool -l "$app/Contents/MacOS/glint-api" | awk '
    $1 == "cmd" { legacy = ($2 == "LC_VERSION_MIN_MACOSX"); modern = ($2 == "LC_BUILD_VERSION") }
    (legacy && $1 == "version") || (modern && $1 == "minos") { print $2; exit }
')"
[[ -n "$minimum_os" ]] || { echo "Cannot read macOS deployment target." >&2; exit 1; }
external="$(otool -L "$app/Contents/MacOS/glint-api" | awk 'NR>1 && $1 !~ /^\/System\/Library\// && $1 !~ /^\/usr\/lib\// {print $1}')"
if [[ -n "$external" ]]; then
    echo "The executable has unbundled non-system dependencies:" >&2
    echo "$external" >&2
    exit 1
fi
plist="$app/Contents/Info.plist"
plutil -create xml1 "$plist"
plutil -insert CFBundleName -string "Glint API" "$plist"
plutil -insert CFBundleDisplayName -string "Glint API" "$plist"
plutil -insert CFBundleIdentifier -string "dev.gpui-glint.api" "$plist"
plutil -insert CFBundleExecutable -string "glint-api" "$plist"
plutil -insert CFBundlePackageType -string "APPL" "$plist"
plutil -insert CFBundleShortVersionString -string "$version" "$plist"
plutil -insert CFBundleVersion -string "$version" "$plist"
plutil -insert CFBundleIconFile -string "glint-api.icns" "$plist"
plutil -insert LSApplicationCategoryType -string "public.app-category.developer-tools" "$plist"
plutil -insert LSMinimumSystemVersion -string "$minimum_os" "$plist"
plutil -insert NSHighResolutionCapable -bool YES "$plist"
plutil -insert NSPrincipalClass -string "NSApplication" "$plist"
plutil -lint "$plist"
iconset="$work/glint-api.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" crates/glint-api/assets/glint-api-icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" crates/glint-api/assets/glint-api-icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/glint-api.icns"
identity="${GLINT_API_CODESIGN_IDENTITY:--}"
if [[ "$identity" == - ]]; then
    codesign --force --sign - --timestamp=none "$app"
    signing_note="此包使用本地 ad-hoc 签名，未做 Developer ID 签名或 Apple 公证。"
else
    codesign --force --sign "$identity" --options runtime --timestamp "$app"
    signing_note="此包已使用指定证书签名；此脚本不提交 Apple 公证。"
fi
codesign --verify --deep --strict --verbose=2 "$app"
[[ "$("$app/Contents/MacOS/glint-api" --version)" == "Glint API $version" ]] || { echo "Bundle version check failed." >&2; exit 1; }
ln -s /Applications "$stage/Applications"
cat > "$stage/安装说明.txt" <<INSTALL
Glint API $version · macOS $arch

将「Glint API.app」拖到「Applications」安装，再从应用程序目录打开。
需要 macOS $minimum_os 或更高版本。架构：${arch}（不是 Universal 通用包）。

$signing_note

工作区保存在 ~/Library/Application Support/Glint API/workspace.sqlite3。
替换或移除应用不会删除已有工作区。

MCP：在应用中按 Cmd+K，选择「复制 MCP 连接配置」。
命令行入口：/Applications/Glint API.app/Contents/MacOS/glint-api --mcp
INSTALL
image="$output/Glint-API-$version-macos-$arch.dmg"
hdiutil create -volname "Glint API" -srcfolder "$stage" -format UDZO -ov "$image"
hdiutil verify "$image"
mkdir "$work/mounted"
hdiutil attach "$image" -readonly -nobrowse -mountpoint "$work/mounted"
mounted=true
installed="$work/mounted/Glint API.app"
codesign --verify --deep --strict --verbose=2 "$installed"
[[ "$("$installed/Contents/MacOS/glint-api" --version)" == "Glint API $version" ]] || { echo "Mounted DMG version check failed." >&2; exit 1; }
[[ -s "$installed/Contents/Resources/glint-api.icns" && -L "$work/mounted/Applications" ]] || { echo "Mounted DMG installation layout is invalid." >&2; exit 1; }
hdiutil detach "$work/mounted"
mounted=false
# Publish a directly usable .app as well as the DMG. This directory is build output.
rm -rf "$output/Glint API.app"
ditto "$app" "$output/Glint API.app"
(
    cd "$output"
    shasum -a 256 "$(basename "$image")" > "$(basename "$image").sha256"
)
printf 'Application: %s\nInstaller: %s\nSHA-256: %s.sha256\n' "$output/Glint API.app" "$image" "$image"
