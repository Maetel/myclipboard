#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
export PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.cargo/bin:$PATH"
cd "$script_dir"

if ! command -v npm >/dev/null 2>&1; then
  print "Node.js를 찾을 수 없습니다. Homebrew로 Node.js를 설치해 주세요."
  read -r "?Enter를 누르면 닫힙니다."
  exit 1
fi

if [[ ! -d node_modules ]]; then
  npm install
fi

exec npm run desktop:dev
