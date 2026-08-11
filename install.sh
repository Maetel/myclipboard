#!/usr/bin/env bash
set -euo pipefail

[[ "$(uname -s)" == Darwin ]] || {
  echo "이 설치 명령은 macOS에서 실행해 주세요." >&2
  exit 1
}

for tool in /usr/bin/curl /usr/bin/tar; do
  [[ -x "$tool" ]] || {
    echo "필요한 macOS 기본 도구를 찾지 못했습니다: $tool" >&2
    exit 1
  }
done

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/mymemo-clipboard-bootstrap.XXXXXX")"
archive="$work_dir/source.tar.gz"
source_dir="$work_dir/source"

cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT

/usr/bin/curl \
  --proto '=https' \
  --tlsv1.2 \
  --fail \
  --location \
  --silent \
  --show-error \
  --output "$archive" \
  'https://codeload.github.com/Maetel/myclipboard/tar.gz/refs/heads/main'

mkdir -p -- "$source_dir"
/usr/bin/tar -xzf "$archive" -C "$source_dir" --strip-components=1

installer="$source_dir/scripts/install-macos.sh"
package_json="$source_dir/package.json"
[[ -f "$installer" && ! -L "$installer" && -f "$package_json" && ! -L "$package_json" ]] || {
  echo "받은 파일에 MyMemo Clipboard 설치 프로그램이 없습니다." >&2
  exit 1
}

if ! /usr/bin/grep -Eq '"name"[[:space:]]*:[[:space:]]*"mymemo-clipboard"' "$package_json"; then
  echo "받은 파일이 MyMemo Clipboard 소스가 아닙니다." >&2
  exit 1
fi

bash "$installer" "$@"
