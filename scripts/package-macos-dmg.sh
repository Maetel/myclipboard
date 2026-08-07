#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app_path="${1:-$repo_root/src-tauri/target/release/bundle/macos/MyMemo Clipboard.app}"
output_path="${2:-$repo_root/release/MyMemo Clipboard_0.1.0_aarch64.dmg}"

[[ "$(uname -s)" == Darwin ]] || { echo "macOS is required" >&2; exit 2; }
[[ -d "$app_path" && ! -L "$app_path" ]] || { echo "app bundle not found: $app_path" >&2; exit 2; }

bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app_path/Contents/Info.plist")"
[[ "$bundle_id" == my.memos.clipboard ]] || { echo "unexpected bundle identifier: $bundle_id" >&2; exit 2; }

mkdir -p "$(dirname "$output_path")"
stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/mymemo-clipboard-dmg.XXXXXX")"
cleanup() { rm -rf -- "$stage_dir"; }
trap cleanup EXIT

/usr/bin/ditto "$app_path" "$stage_dir/MyMemo Clipboard.app"
ln -s /Applications "$stage_dir/Applications"

/usr/bin/hdiutil create \
  -volname "MyMemo Clipboard" \
  -srcfolder "$stage_dir" \
  -format UDZO \
  -ov \
  "$output_path"
/usr/bin/hdiutil verify "$output_path"

printf 'DMG=%s\n' "$output_path"
