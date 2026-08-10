# MyMemo Clipboard 사용성 감사와 개선 기준

이 문서는 2026-08-10 현재 앱을 기획자·사용자·테스터 관점에서 다시 살펴본 결과입니다. 특히 macOS에서 저장된 계정으로 시작할 때 로그인 화면이 보이는 문제, 기록 창을 여는 동안 앱이 멈춘 것처럼 보이는 문제, 원격 파일을 선택한 뒤 최대 45초 동안 조작할 수 없는 문제를 우선 대상으로 삼습니다.

## 판단 기준

- 사용자가 기다려야 하는 작업은 화면 그리기와 입력 처리를 막지 않습니다. Tauri도 무거운 작업에는 async command를 권장하며, 동기 command는 기본적으로 main thread에서 실행된다고 설명합니다. blocking 파일·네트워크 작업은 전용 executor에서 수행합니다. ([Tauri async commands](https://v2.tauri.app/develop/calling-rust/), [`spawn_blocking`](https://docs.rs/tauri/latest/tauri/async_runtime/fn.spawn_blocking.html))
- 소요 시간을 알 수 없는 동기화나 파일 요청에는 움직이는 진행 표시와 구체적인 상태를 보여 줍니다. 중단해도 안전한 작업은 취소할 수 있어야 합니다. ([Apple progress indicators](https://developer.apple.com/design/human-interface-guidelines/progress-indicators))
- 동적으로 바뀌는 목록은 갱신 중임을 `aria-busy`로 전달하고, 완료된 상태를 `role=status`에서 한 번 알립니다. ([MDN `aria-busy`](https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Reference/Attributes/aria-busy), [MDN live regions](https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Guides/Live_regions))
- 사용자가 이미 알려 준 상태를 다시 묻지 않습니다. 유효성 검사는 첫 화면을 막지 않고 백그라운드 동기화 결과로 반영합니다.
- 목록의 한 번 클릭은 선택, Enter나 더블클릭은 실행으로 통일합니다. 느린 실행 중 같은 요청을 다시 만들지 않습니다.
- 같은 앱을 다시 열어도 process와 동기화 worker는 하나만 유지하고 기존 window를 앞으로 가져옵니다. ([Tauri Single Instance](https://v2.tauri.app/plugin/single-instance/))

현재 frontend는 이미 Vite 기반 CSR입니다. 멈춤의 원인은 rendering 방식이 아니라 동기식 Rust IPC와 한꺼번에 시작하는 후속 요청이므로 framework를 교체하지 않고 작업 경계를 비동기로 바꿉니다.

## 발견한 문제와 결정

| 우선순위 | 사용자가 겪는 문제 | 확인한 원인 | 개선 결정 | 완료 증거 |
| --- | --- | --- | --- | --- |
| P0 | 저장된 계정이 있어도 로그인 화면이 먼저 보이거나 시작이 늦음 | 로그인 panel이 기본 노출되고 `load_settings`의 `/me` 요청뿐 아니라 Tauri `setup`의 Keychain·단축키 복원이 main thread를 막음 | 시작 panel만 먼저 보이고, Keychain·단축키 복원과 session 확인은 worker에서 수행. 저장 session이 있으면 계정 화면으로 전환하고 session 폐기는 background sync에서 처리 | 저장 session 유무별 초기 DOM·command 검증, Mac main-thread sample·재실행 |
| P0 | Mac에서 기록 창을 열면 앱이 멈춘 것처럼 보임 | `clipboard_history`가 암호화 파일 읽기·복호화를 main thread command에서 수행 | 기록·썸네일 command를 async + blocking worker로 이동하고 목록에 loading/오류/재시도 상태 제공 | UI heartbeat와 목록 열기 실기기 확인 |
| P0 | 원격 파일을 더블클릭하면 최대 45초 동안 앱이 멈춤 | `clipboard_select`가 polling·download·파일 쓰기·sleep을 main thread에서 수행 | 전체 선택 작업을 blocking worker로 이동. 행 spinner·경과 시간·Escape 취소를 유지하고 중복 실행 차단 | 대기 중 검색창·Escape 반응, 지연 응답 뒤 붙여넣기 없음 |
| P1 | 파일이 텍스트와 비슷하게 보여 무엇인지 바로 알기 어려움 | 종류는 작은 meta 문구에만 표시 | 파일 행과 최근 기록에 색상 이외의 파일 icon과 `파일` 문구를 함께 표시 | DOM·접근성 이름 확인 |
| P1 | 팝업을 열 때 썸네일 요청이 한꺼번에 발생 | 최대 500개 행을 만들며 모든 thumbnail command를 즉시 호출 | viewport에 들어온 썸네일만 lazy load | 초기 요청 수와 scroll 후 요청 확인 |
| P1 | 여러 갱신이 겹치면 목록이 뒤로 돌아가거나 선택·scroll이 튐 | open/update event마다 제한 없이 `refresh`; 늦은 응답 구분 없음 | 한 번에 하나만 갱신하고 마지막 요청만 반영. 내용이 같으면 DOM을 유지 | 역순 응답·event burst 회귀 검사 |
| P1 | 한 번 클릭만으로 즉시 붙여넣혀 실수하기 쉬움 | 행 `click`이 바로 실행되지만 footer는 Enter만 안내 | 한 번 클릭은 선택, 더블클릭과 Enter는 실행. 실행 중 다른 행은 비활성 | click/double-click/Enter 조작 검사 |
| P1 | 받은 파일 이름 앞에 내부 item ID가 붙을 수 있음 | download 파일을 `{item_id}-{filename}`으로 생성 | item별 내부 directory 안에 원래 basename으로 저장 | Finder에 붙인 파일명 확인 |
| P2 | 초기 로딩 실패 시 빈 목록처럼 보이고 다시 시도할 방법이 없음 | `refresh()` rejection을 처리하지 않음 | 오류 문구와 `다시 시도` 버튼 제공, 마지막 정상 목록 유지 | 강제 오류 후 retry 검사 |
| P2 | 설정·로그인·로그아웃·수동 동기화 중에도 창 전체가 멈출 수 있음 | blocking HTTP/keyring/disk command가 모두 동기 함수 | 사용자 조작 command 전체를 async worker로 이동하고 button busy 상태 통일 | 느린 응답 중 window interaction 확인 |
| P2 | popup이 동기화 event를 연속으로 받으면 선택 중인 행까지 다시 그림 | 선택 중에도 `clipboard-updated`가 즉시 refresh | 실행 중 갱신을 보류하고 실패 후 한 번만 반영 | 선택 중 event burst 검사 |
| P1 | 앱을 다시 열면 같은 계정으로 process가 여러 개 생김 | single-instance 보장이 없음 | 두 번째 실행은 종료하고 기존 main window를 앞으로 가져옴 | Mac·Windows에서 재실행 후 process 1개 확인 |

## 이번 개선 범위 밖에서 추적할 항목

- 원격 파일의 정확한 byte 진행률은 현재 서버 API가 길이 기반 stream progress를 제공하지 않아 표시할 수 없습니다. 이번에는 경과 시간과 취소 가능 상태를 정확히 보여 주고, 추후 channel/stream 방식에서 실제 진행률을 추가합니다.
- Keychain 권한 prompt 자체는 운영체제가 소유합니다. prompt가 필요한 경우에도 WebView main thread를 막지 않도록 읽기를 worker에서 수행합니다.
- macOS 서명·notarization과 최초 실행 권한 안내는 별도 release gate입니다. 기능 응답성 개선 완료와 배포 신뢰 완료를 같은 것으로 표시하지 않습니다.

## 완료 조건

1. 저장 session이 있으면 로그인 form을 한 번도 보여 주지 않고 계정 화면으로 이동합니다.
2. 기록·썸네일·파일 전송·로그인 관련 blocking 작업이 main thread command에 남지 않습니다.
3. 기록 loading과 파일 받기 중 창 애니메이션·검색 입력·Escape가 반응합니다.
4. file 행은 icon, 파일명, 크기, 상대 시각을 함께 표시하고 원래 파일명으로 붙여넣습니다.
5. click은 선택만, Enter·더블클릭은 한 번만 실행합니다.
6. 겹친 refresh, 연속 event, 늦은 thumbnail 응답이 최신 화면을 되돌리지 않습니다.
7. 자동 회귀 검사와 Mac 실기기에서 위 흐름을 확인하고 결과를 `docs/USER_SCENARIOS.md`에 기록합니다.
8. 앱을 두 번 열어도 process는 하나이고 기존 window가 다시 보입니다.
