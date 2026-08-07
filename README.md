# MyMemo Clipboard

Jamserver 계정으로 로그인해 Windows와 macOS 사이에서 텍스트 클립보드 기록을 공유하는 독립 데스크톱 앱입니다.

## 사용 방법

1. Jamserver Admin의 **클립보드** 페이지에서 사용자를 만듭니다.
2. 관리자가 정한 아이디와 비밀번호를 사용자에게 전달합니다.
3. 앱에서 Jamserver 주소, 아이디, 비밀번호를 입력합니다.
4. 같은 계정으로 로그인한 기기에서 텍스트나 HTTP(S) 링크를 복사합니다.
5. `Ctrl+Shift+V`를 눌러 기록을 열고, 방향키와 Enter로 이전 앱에 붙여넣습니다.

비밀번호는 기기에 저장하지 않습니다. 로그인 세션과 로컬 기록 암호화 키는 운영체제 보안 저장소에 보관하며, 로컬 기록과 전송 대기 항목은 AES-256-GCM으로 암호화합니다. 서버는 계정별 기록을 분리하고 7일 또는 최근 500개까지만 보관합니다.
로그인한 사용자는 앱에서 비밀번호를 직접 바꿀 수 있습니다. 변경한 기기의 로그인은 유지되고 다른 기기는 로그아웃됩니다. 관리자가 비밀번호를 바꾸면 모든 기기에서 로그아웃됩니다.

파일과 이미지는 한 항목당 10MiB까지 공유합니다. 처음에는 파일 정보와 이미지 썸네일만 서버로 보내며, 다른 기기에서 붙여넣기를 선택할 때 원본 기기가 온라인이면 원본을 암호화해 잠시 중계합니다. 서버는 원본을 클립보드 기록으로 장기 보관하지 않습니다. 현재는 한 번에 파일 하나 또는 클립보드 이미지 하나를 지원합니다.

## 저장소에서 바로 실행

macOS에서는 저장소의 `MyMemo Clipboard Dev.command`를 더블클릭하면 앱을 개발 모드로 바로 실행합니다. `Applications` 폴더에 복사하거나 매번 데스크톱 빌드를 만들 필요가 없습니다. 처음 한 번만 의존성을 설치하고, 이후 프런트엔드와 Rust 코드 변경을 감지해 자동으로 다시 반영합니다. 터미널에서는 다음 명령과 같습니다.

```bash
npm run desktop:dev
```

## 개발

요구 사항:

- Node.js 22 이상
- Rust stable
- Tauri 2의 [운영체제별 prerequisites](https://v2.tauri.app/start/prerequisites/)

```bash
npm install
npm run typecheck
npm run build
npm run desktop:build
```

Windows에서는 NSIS 설치 파일, macOS에서는 `.app`과 `.dmg`를 만듭니다. macOS 배포본은 실제 배포 전에 Developer ID 서명과 notarization이 필요합니다.
배포용 macOS DMG는 아래 명령으로 다시 패키징합니다. DMG 안에는 앱과
`Applications` 폴더 바로가기가 함께 들어가므로 앱을 바로가기로 드래그해
설치할 수 있습니다.

```bash
npm run desktop:package-macos-dmg
```

## 서버 연동 방식

앱은 입력한 Jamserver origin 아래의 `/admin/clipboard-app/v1`만 사용합니다. 로그인은 아이디·비밀번호·기기 ID를 보내며, 성공하면 90일 세션을 받습니다. 이후 요청은 그 세션을 Bearer 인증으로 전송합니다.

- `POST /login`, `POST /logout`, `POST /password`, `GET /me`
- `GET /spaces`, `POST /items`, `GET /feed`, `DELETE /items/:id`, `GET /items/:id/thumbnail`
- `POST /items/:id/file-requests`, `GET /file-requests/pending`, `GET·PUT /file-requests/:id/content`

서버가 로그인 세션을 폐기하거나 사용자를 중지하면 다음 동기화부터 접근이 거부됩니다. 앱은 서버 응답 본문이나 세션 원문을 로그로 남기지 않습니다.
