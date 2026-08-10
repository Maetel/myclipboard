#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
install_after_build=false

if [[ "${1:-}" == "--install" ]]; then
  install_after_build=true
  shift
fi

if [[ $# -ne 0 ]]; then
  echo "사용법: $0 [--install]" >&2
  exit 2
fi

if [[ -z "${WSL_DISTRO_NAME:-}" ]] || [[ ! -x /mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe ]]; then
  echo "이 명령은 Windows의 WSL 터미널에서 실행해 주세요." >&2
  exit 1
fi

if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=normal)" ]]; then
  echo "저장소에 커밋하지 않은 변경이 있어 Windows 빌드를 중단했습니다." >&2
  echo "변경을 커밋하거나 임시 보관한 뒤 다시 실행해 주세요." >&2
  exit 1
fi

source_commit="$(git -C "$repo_root" rev-parse HEAD)"
powershell=/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe
windows_profile="$($powershell -NoLogo -NoProfile -NonInteractive -Command '[Environment]::GetFolderPath("UserProfile")' | tr -d '\r')"

if [[ -z "$windows_profile" ]]; then
  echo "Windows 사용자 폴더를 확인하지 못했습니다." >&2
  exit 1
fi

windows_home="$(wslpath -u "$windows_profile")"
build_root="${MYMEMO_WINDOWS_BUILD_DIR:-$windows_home/Build/mymemo-clipboard}"
node_exe=/mnt/c/Program\ Files/nodejs/node.exe

if [[ ! -x "$node_exe" ]]; then
  echo "Windows용 Node.js를 찾지 못했습니다. Node.js 22 이상을 Windows에 설치해 주세요." >&2
  exit 1
fi

if [[ ! -x "$windows_home/.cargo/bin/cargo.exe" ]]; then
  echo "Windows용 Rust를 찾지 못했습니다. rustup으로 stable toolchain을 설치해 주세요." >&2
  exit 1
fi

if [[ -e "$build_root" && ! -d "$build_root/.git" ]]; then
  echo "빌드 경로가 다른 파일에 사용 중입니다: $build_root" >&2
  exit 1
fi

if [[ ! -d "$build_root/.git" ]]; then
  mkdir -p "$(dirname "$build_root")"
  git clone "$repo_root" "$build_root"
  git -C "$build_root" config mymemo.managedBuild true
elif [[ "$(git -C "$build_root" config --bool --get mymemo.managedBuild || true)" != "true" ]]; then
  echo "빌드 경로가 MyMemo Clipboard 전용 복사본이 아닙니다: $build_root" >&2
  echo "다른 빈 경로를 MYMEMO_WINDOWS_BUILD_DIR에 지정해 주세요." >&2
  exit 1
fi

if [[ -n "$(git -C "$build_root" status --porcelain --untracked-files=normal)" ]]; then
  echo "Windows 빌드용 복사본에 커밋하지 않은 변경이 있어 중단했습니다: $build_root" >&2
  exit 1
fi

git -C "$build_root" fetch --no-tags "$repo_root" "$source_commit"
git -C "$build_root" checkout --detach --force "$source_commit"

build_root_windows="$(wslpath -w "$build_root")"
runner_windows="$(wslpath -w "$build_root/scripts/windows-build-runner.mjs")"

runner_args=("$runner_windows" "--repo" "$build_root_windows")
if [[ "$install_after_build" == true ]]; then
  runner_args+=("--install")
fi

"$node_exe" "${runner_args[@]}"
