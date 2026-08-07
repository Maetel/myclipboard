#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
export PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.cargo/bin:$PATH"
cd "$script_dir"

dependencies_changed=false
if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  branch=$(git branch --show-current)
  if [[ "$branch" != "main" ]]; then
    print "현재 브랜치가 main이 아니어서 자동 업데이트하지 않았습니다: ${branch:-분리된 checkout}"
  elif [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
    print "로컬 수정 사항이 있어서 자동 업데이트를 건너뜁니다. 파일은 그대로 보존됩니다."
  else
    before_update=$(git rev-parse HEAD)
    print "최신 코드를 확인합니다…"
    if git pull --ff-only origin main; then
      after_update=$(git rev-parse HEAD)
      if [[ "$before_update" != "$after_update" ]] &&
        ! git diff --quiet "$before_update" "$after_update" -- package.json package-lock.json; then
        dependencies_changed=true
      fi
    else
      print "업데이트하지 못했습니다. 현재 받아 둔 코드로 계속 실행합니다."
    fi
  fi
fi

if ! command -v npm >/dev/null 2>&1; then
  print "Node.js를 찾을 수 없습니다. Homebrew로 Node.js를 설치해 주세요."
  read -r "?Enter를 누르면 닫힙니다."
  exit 1
fi

if [[ ! -d node_modules || "$dependencies_changed" == true ]]; then
  npm install
fi

exec npm run desktop:dev
