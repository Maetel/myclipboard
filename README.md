# MyMemo Clipboard

마이메모(`memos.my`) 계정으로 로그인해 웹과 여러 Windows/macOS 기기에서 클립보드 기록을 공유하는 독립 데스크톱 앱입니다.

## 사용 방법

1. `https://memos.my/login`에서 자유롭게 가입합니다.
2. 앱에서 같은 아이디와 비밀번호로 로그인합니다.
3. 텍스트, HTTP(S) 링크, 이미지 또는 일반 파일 하나를 복사합니다.
4. 마이메모 웹의 **기기 간 클립보드**에서 기록을 확인하거나, Windows에서는 `Ctrl+Shift+V`, macOS에서는 `Command+Shift+V`를 눌러 붙여넣습니다.
5. 기록 창에서 방향키나 한 번 클릭으로 항목을 고르고, Enter나 더블클릭으로 붙여넣습니다. 마음이 바뀌면 Escape를 누르거나 창 밖을 클릭하면 아무것도 붙여넣지 않고 닫힙니다.

비밀번호는 기기에 저장하지 않습니다. Windows에서는 로그인 세션과 로컬 기록 암호화 키를 운영체제 보안 저장소에 보관합니다. macOS에서는 앱을 다시 빌드할 때마다 Keychain이 사용자 로그인 비밀번호를 요구하지 않도록, 앱 전용 사용자 폴더 안의 소유자 전용 디렉터리와 파일(`0700`/`0600`)에 보관합니다. 로컬 기록과 전송 대기 항목은 AES-256-GCM으로 암호화합니다. 서버는 계정별 기록을 분리하고 7일 또는 최근 500개까지만 보관합니다.
앱은 Claude나 OpenUsage와 연동하지 않으며 관련 자격증명을 조회하지 않습니다. macOS 0.2.4로 처음 올린 뒤에는 예전 Keychain 세션을 자동으로 읽지 않으므로 마이메모 계정으로 한 번 다시 로그인해야 하지만, macOS 사용자 로그인 비밀번호 창은 더 이상 열리지 않습니다.
로그인한 사용자는 앱에서 마이메모 비밀번호를 직접 바꿀 수 있습니다. 변경한 기기의 로그인은 유지되고 다른 기기는 로그아웃됩니다. 기본 저장 용량은 메모와 클립보드를 합쳐 50MiB이며 `stan` 계정은 제한이 없습니다.

파일과 이미지는 한 항목당 10MiB까지 공유합니다. Windows에서 일반 파일을 복사하면 경로·크기·수정 시각과 파일 정보만 기록하며 원본 바이트를 읽거나 hash·thumbnail을 만들지 않습니다. 다른 기기에서 붙여넣기를 선택한 뒤에만 원본 기기가 파일을 읽어 암호화된 임시 중계를 시작합니다. 서버와 두 앱은 SHA-256을 각각 확인하며 서버는 원본을 클립보드 기록으로 장기 보관하지 않습니다. 폴더와 여러 파일은 재귀 탐색하지 않고 대상에서 제외합니다. 현재는 한 번에 파일 하나 또는 클립보드 이미지 하나를 지원합니다.

Windows 앱은 클립보드 변경 알림과 `Ctrl+Shift+V` 팝업 열기에서만 작업을 시작합니다. 시작할 때는 WebView2를 만들지 않고, 설정이나 클립보드 기록을 실제로 열 때만 생성합니다. 창을 닫으면 WebView2도 제거하고, 동기화가 끝난 뒤 유휴 HTTP 연결을 유지하지 않습니다. 유휴 상태에서는 500ms clipboard 확인, 3초 feed·`/spaces` 확인, shell 하위 프로세스를 실행하지 않습니다.

macOS에서는 기록·썸네일·파일 전송과 로컬 로그인 정보 확인을 화면 처리와 분리해, 느린 작업 중에도 창과 Escape 입력이 반응하도록 구성합니다. 저장된 로그인 정보가 있으면 로그인 화면을 먼저 보여 주지 않고 계정 화면으로 이동합니다. 파일은 문자열 경로가 아닌 macOS 파일 목록 Clipboard 형식으로 공유합니다.

Windows가 Clipboard 기록이나 다른 기기 동기화에서 제외하라고 표시한 내용은 앱도 캡처하지 않습니다. 릴리스 앱의 로그인·동기화 origin은 exact `https://memos.my`로 고정하며, 로그인 화면에서 다른 서버로 계정 정보를 보내는 설정은 제공하지 않습니다.

## 저장소에서 바로 실행

macOS에서는 저장소의 `MyMemo Clipboard Dev.command`를 더블클릭하면 앱을 개발 모드로 바로 실행합니다. `Applications` 폴더에 복사하거나 매번 데스크톱 빌드를 만들 필요가 없습니다. 실행할 때마다 `main`의 최신 코드를 `git pull --ff-only`로 확인하고, 필요한 경우 의존성도 다시 설치합니다. 로컬에서 수정 중인 파일이 있거나 다른 브랜치에 있으면 파일을 덮어쓰지 않고 자동 업데이트만 건너뜁니다. 네트워크 문제로 업데이트하지 못해도 이미 받아 둔 코드로 실행합니다.

처음 한 번만 전체 의존성을 설치하고, 이후 프런트엔드와 Rust 코드 변경을 감지해 자동으로 다시 반영합니다. 터미널에서는 다음 명령과 같습니다.

```bash
npm run desktop:dev
```

macOS 설치용 `.app`과 `.dmg`를 만들려면 다음 명령을 사용합니다.

```bash
npm ci
npm run desktop:build:macos
```

Windows 앱은 WSL 터미널에서 다음 세 명령으로 최신 코드를 받고, 검증된 설치 파일을 만든 뒤 현재 사용자에게 설치하고 실행할 수 있습니다.

```bash
git switch main
git pull --ff-only origin main
npm run desktop:install:windows
```

이 명령은 WSL 경로를 Windows 컴파일러에 직접 넘기지 않습니다. 현재 commit의 깨끗한 복사본을 Windows 사용자 폴더의 `Build/mymemo-clipboard`에 준비한 뒤 `npm ci`, TypeScript·frontend 검사, Windows Rust test, NSIS build를 차례로 통과시킵니다. 기존에 실행 중인 MyMemo Clipboard를 종료하고 새 버전을 설치한 뒤 다시 실행합니다. 설치하지 않고 NSIS 파일만 만들려면 `npm run desktop:build:windows`를 사용합니다. 별도 빌드 경로가 필요하면 `MYMEMO_WINDOWS_BUILD_DIR`에 WSL 경로를 지정할 수 있습니다.

## 개발

요구 사항:

- Node.js 22 이상
- Rust stable
- Tauri 2의 [운영체제별 prerequisites](https://v2.tauri.app/start/prerequisites/)

```bash
npm ci
npm run typecheck
npm run build
npm run desktop:build
```

`npm run desktop:build`는 Tauri의 기본 build입니다. Windows용 NSIS 설치 파일을 WSL에서 만들 때는 위의 `desktop:build:windows`, macOS용 `.app`과 `.dmg`를 만들 때는 `desktop:build:macos`를 사용합니다. macOS 배포본은 실제 배포 전에 Developer ID 서명과 notarization이 필요합니다.
배포용 macOS DMG는 아래 명령으로 다시 패키징합니다. DMG 안에는 앱과
`Applications` 폴더 바로가기가 함께 들어가므로 앱을 바로가기로 드래그해
설치할 수 있습니다.

```bash
npm run desktop:package-macos-dmg
```

## 서버 연동 방식

앱은 마이메모 origin의 `/api/clipboard/v1`만 사용합니다. 로그인은 아이디·비밀번호·기기 ID를 보내며, 성공하면 `smc_` 형식의 90일 앱 전용 세션을 받습니다. 이후 요청은 그 세션을 Bearer 인증으로 전송합니다.

- `POST /login`, `POST /logout`, `POST /password`, `GET /me`
- `GET /spaces`, `POST /items`, `GET /feed`, `DELETE /items/:id`, `GET /items/:id/thumbnail`
- `POST /items/:id/file-requests`, `GET /file-requests/pending`, `GET·PUT /file-requests/:id/content`

서버가 로그인 세션을 폐기하거나 사용자를 중지하면 다음 동기화부터 접근이 거부됩니다. 앱은 서버 응답 본문이나 세션 원문을 로그로 남기지 않습니다.

구체적인 Windows 사용 흐름과 검증 기준은 [`docs/USER_SCENARIOS.md`](docs/USER_SCENARIOS.md)에 정리되어 있습니다.
버전이 고정된 요청·응답 형식은 [`contracts/clipboard-v1.openapi.json`](contracts/clipboard-v1.openapi.json)에 있으며 SmallMemo 저장소의 같은 파일과 byte 단위로 일치해야 합니다. 두 저장소는 이 문서 외의 source나 package를 공유하지 않습니다.
