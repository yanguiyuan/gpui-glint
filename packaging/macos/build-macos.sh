#!/bin/bash
# Build a native release .app and a drag-to-Applications DMG on macOS.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo"
product="${1:-workbench}"
case "$product" in
    workbench)
        binary=glint-workbench
        display_name="Glint Workbench"
        bundle_id=dev.gpui-glint.workbench
        artifact=Glint-Workbench
        work_subdir=macos-workbench-installer
        identity="${GLINT_CODESIGN_IDENTITY:-${GLINT_WORKBENCH_CODESIGN_IDENTITY:--}}"
        agent_note="Agent 会话保存在 ~/Library/Application Support/Glint Agent/。"
        ;;
    api)
        binary=glint-api
        display_name="Glint API"
        bundle_id=dev.gpui-glint.api
        artifact=Glint-API
        work_subdir=macos-installer
        identity="${GLINT_CODESIGN_IDENTITY:-${GLINT_API_CODESIGN_IDENTITY:--}}"
        agent_note=""
        ;;
    *) echo "Unknown product: $product (expected workbench or api)." >&2; exit 1 ;;
esac
[[ $# -le 1 ]] || { echo "Usage: $0 [workbench|api]" >&2; exit 1; }
if [[ "$(uname -s)" != Darwin ]]; then
    echo "Build this installer on macOS." >&2
    exit 1
fi
for tool in cargo rustc plutil xcrun sips iconutil codesign hdiutil ditto shasum otool strip; do
    command -v "$tool" >/dev/null || { echo "Required tool is missing: $tool" >&2; exit 1; }
done
version="$(awk -F '"' '/^version[[:space:]]*=/ { print $2; exit }' "crates/$binary/Cargo.toml")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Use a stable application version." >&2; exit 1; }
host="$(rustc -vV | awk '/^host:/ {print $2}')"
case "$host" in
    aarch64-apple-darwin) arch=arm64 ;;
    x86_64-apple-darwin) arch=x86_64 ;;
    *) echo "Unsupported macOS Rust host: $host" >&2; exit 1 ;;
esac
# An explicit native target also avoids an unrelated build.target in user Cargo config.
cargo build -p "$binary" --release --locked --target "$host"
target_dir="$(cargo metadata --no-deps --format-version 1 | plutil -extract target_directory raw -o - -)"
[[ -n "$target_dir" && "$target_dir" != / ]] || { echo "Invalid Cargo target directory." >&2; exit 1; }
work_root="$target_dir/$work_subdir"
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
app="$stage/$display_name.app"
resources="$app/Contents/Resources"
mkdir -p "$app/Contents/MacOS" "$resources"
cp "$target_dir/$host/release/$binary" "$app/Contents/MacOS/$binary"
chmod 755 "$app/Contents/MacOS/$binary"
strip -x "$app/Contents/MacOS/$binary"
# Intel binaries may use LC_VERSION_MIN_MACOSX; Apple Silicon uses LC_BUILD_VERSION.
minimum_os="$(otool -l "$app/Contents/MacOS/$binary" | awk '
    $1 == "cmd" { legacy = ($2 == "LC_VERSION_MIN_MACOSX"); modern = ($2 == "LC_BUILD_VERSION") }
    !found && ((legacy && $1 == "version") || (modern && $1 == "minos")) { minimum = $2; found = 1 }
    END { print minimum }
')"
[[ -n "$minimum_os" ]] || { echo "Cannot read macOS deployment target." >&2; exit 1; }
external="$(otool -L "$app/Contents/MacOS/$binary" | awk 'NR>1 && $1 !~ /^\/System\/Library\// && $1 !~ /^\/usr\/lib\// {print $1}')"
if [[ -n "$external" ]]; then
    echo "The executable has unbundled non-system dependencies:" >&2
    echo "$external" >&2
    exit 1
fi
plist="$app/Contents/Info.plist"
plutil -create xml1 "$plist"
plutil -insert CFBundleName -string "$display_name" "$plist"
plutil -insert CFBundleDisplayName -string "$display_name" "$plist"
plutil -insert CFBundleIdentifier -string "$bundle_id" "$plist"
plutil -insert CFBundleExecutable -string "$binary" "$plist"
plutil -insert CFBundlePackageType -string "APPL" "$plist"
plutil -insert CFBundleShortVersionString -string "$version" "$plist"
plutil -insert CFBundleVersion -string "$version" "$plist"
plutil -insert CFBundleIconFile -string "$binary.icns" "$plist"
plutil -insert LSApplicationCategoryType -string "public.app-category.developer-tools" "$plist"
plutil -insert LSMinimumSystemVersion -string "$minimum_os" "$plist"
plutil -insert NSHighResolutionCapable -bool YES "$plist"
plutil -insert NSPrincipalClass -string "NSApplication" "$plist"
plutil -lint "$plist"
iconset="$work/$binary.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" crates/glint-api/assets/glint-api-icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" crates/glint-api/assets/glint-api-icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/$binary.icns"
if [[ "$identity" == - ]]; then
    codesign --force --sign - --timestamp=none "$app"
    signing_note="此包使用本地 ad-hoc 签名，未做 Developer ID 签名或 Apple 公证。"
else
    codesign --force --sign "$identity" --options runtime --timestamp "$app"
    signing_note="此包已使用指定证书签名；此脚本不提交 Apple 公证。"
fi
codesign --verify --deep --strict --verbose=2 "$app"
[[ "$("$app/Contents/MacOS/$binary" --version)" == "$display_name $version" ]] || { echo "Bundle version check failed." >&2; exit 1; }
ln -s /Applications "$stage/Applications"
cat > "$stage/安装说明.txt" <<INSTALL
$display_name $version · macOS $arch

将「$display_name.app」拖到「Applications」安装，再从应用程序目录打开。
需要 macOS $minimum_os 或更高版本。架构：${arch}（不是 Universal 通用包）。

$signing_note

工作区保存在 ~/Library/Application Support/Glint API/workspace.sqlite3。
$agent_note
替换或移除应用不会删除已有工作区。

MCP：在应用中按 Cmd+K，选择「复制 MCP 连接配置」。
命令行入口：/Applications/$display_name.app/Contents/MacOS/$binary --mcp
INSTALL
image="$output/$artifact-$version-macos-$arch.dmg"
hdiutil create -volname "$display_name" -srcfolder "$stage" -format UDZO -ov "$image"
hdiutil verify "$image"
mkdir "$work/mounted"
hdiutil attach "$image" -readonly -nobrowse -mountpoint "$work/mounted"
mounted=true
installed="$work/mounted/$display_name.app"
codesign --verify --deep --strict --verbose=2 "$installed"
[[ "$("$installed/Contents/MacOS/$binary" --version)" == "$display_name $version" ]] || { echo "Mounted DMG version check failed." >&2; exit 1; }
[[ -s "$installed/Contents/Resources/$binary.icns" && -L "$work/mounted/Applications" ]] || { echo "Mounted DMG installation layout is invalid." >&2; exit 1; }
hdiutil detach "$work/mounted"
mounted=false
# Publish a directly usable .app as well as the DMG. This directory is build output.
rm -rf "$output/$display_name.app"
ditto "$app" "$output/$display_name.app"
(
    cd "$output"
    shasum -a 256 "$(basename "$image")" > "$(basename "$image").sha256"
)
printf 'Application: %s\nInstaller: %s\nSHA-256: %s.sha256\n' "$output/$display_name.app" "$image" "$image"
