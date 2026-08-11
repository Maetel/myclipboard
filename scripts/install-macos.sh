#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app_path=""
launch_after_install=true

usage() {
  cat <<'EOF'
사용법: scripts/install-macos.sh [--app <MyMemo Clipboard.app>] [--no-launch]

옵션 없이 실행하면 현재 소스를 빌드해 ~/Applications에 설치하고 실행합니다.
--app을 지정하면 이미 빌드된 앱을 설치합니다.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      [[ $# -ge 2 ]] || { echo "--app 뒤에 앱 경로가 필요합니다." >&2; exit 2; }
      app_path="$2"
      shift 2
      ;;
    --no-launch)
      launch_after_install=false
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

[[ "$(uname -s)" == Darwin ]] || {
  echo "이 명령은 macOS에서 실행해 주세요." >&2
  exit 1
}

if [[ -z "$app_path" ]]; then
  command -v npm >/dev/null 2>&1 || {
    echo "Node.js 22 이상과 npm을 먼저 설치해 주세요." >&2
    exit 1
  }
  (
    cd "$repo_root"
    npm ci
    npm run tauri -- build --config src-tauri/tauri.macos.conf.json
  )
  app_path="$repo_root/src-tauri/target/release/bundle/macos/MyMemo Clipboard.app"
fi

[[ -d "$app_path" && ! -L "$app_path" ]] || {
  echo "설치할 앱을 찾지 못했습니다: $app_path" >&2
  exit 1
}

info_plist="$app_path/Contents/Info.plist"
[[ -f "$info_plist" && ! -L "$info_plist" ]] || {
  echo "올바른 macOS 앱이 아닙니다: $app_path" >&2
  exit 1
}

bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$info_plist")"
[[ "$bundle_id" == "my.memos.clipboard" ]] || {
  echo "다른 앱은 설치하지 않습니다: $bundle_id" >&2
  exit 1
}

install_root="$HOME/Applications"
mkdir -p -- "$install_root"
[[ -d "$install_root" && ! -L "$install_root" ]] || {
  echo "설치 폴더가 실제 디렉터리가 아닙니다: $install_root" >&2
  exit 1
}

destination="$install_root/MyMemo Clipboard.app"
if [[ -e "$destination" || -L "$destination" ]]; then
  [[ -d "$destination" && ! -L "$destination" ]] || {
    echo "기존 설치 경로가 앱 디렉터리가 아닙니다: $destination" >&2
    exit 1
  }
  existing_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$destination/Contents/Info.plist" 2>/dev/null || true)"
  [[ "$existing_id" == "my.memos.clipboard" ]] || {
    echo "기존 설치 경로에 다른 앱이 있습니다: $destination" >&2
    exit 1
  }
fi

stage_dir="$(mktemp -d "$install_root/.mymemo-clipboard-install.XXXXXX")"
staged_app="$stage_dir/MyMemo Clipboard.app"
previous_app="$stage_dir/Previous MyMemo Clipboard.app"

cleanup() {
  if [[ -d "$previous_app" && ! -e "$destination" ]]; then
    mv -- "$previous_app" "$destination" || true
  fi
  rm -rf -- "$stage_dir"
}
trap cleanup EXIT

/usr/bin/ditto "$app_path" "$staged_app"
staged_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$staged_app/Contents/Info.plist")"
[[ "$staged_id" == "my.memos.clipboard" ]] || {
  echo "복사된 앱의 식별자가 일치하지 않습니다." >&2
  exit 1
}

/usr/bin/osascript -e 'tell application id "my.memos.clipboard" to quit' >/dev/null 2>&1 || true

if [[ -d "$destination" ]]; then
  mv -- "$destination" "$previous_app"
fi
mv -- "$staged_app" "$destination"

if [[ "$launch_after_install" == true ]]; then
  if ! /usr/bin/open "$destination"; then
    echo "설치는 끝났지만 앱을 자동으로 열지 못했습니다: $destination" >&2
  fi
fi

printf '설치 완료: %s\n' "$destination"
