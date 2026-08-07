use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
#[cfg(target_os = "macos")]
use std::{
    io::Write,
    process::{Command, Stdio},
};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use super::Settings;

#[cfg(test)]
const DEFAULT_SHORTCUT: &str = "Ctrl+Shift+V";
const MAX_ITEMS: usize = 500;
const MAX_TEXT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub seq: i64,
    pub id: String,
    pub space_id: String,
    pub origin_device_id: String,
    pub kind: String,
    pub text: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingItem {
    client_event_id: String,
    kind: String,
    text: String,
    local_id: String,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LocalState {
    cursor: i64,
    #[serde(default)]
    items: Vec<ClipboardItem>,
    #[serde(default)]
    pending: Vec<PendingItem>,
}

#[derive(Debug, Deserialize)]
struct Feed {
    events: Vec<Value>,
    next_cursor: i64,
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Clone)]
enum ForegroundTarget {
    #[cfg(target_os = "windows")]
    Windows(isize),
    #[cfg(target_os = "macos")]
    Mac(String),
    None,
}

static FOREGROUND: OnceLock<Mutex<ForegroundTarget>> = OnceLock::new();
static LAST_APPLIED_HASH: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
static SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static POPUP_ARMED: AtomicBool = AtomicBool::new(false);

fn foreground() -> &'static Mutex<ForegroundTarget> {
    FOREGROUND.get_or_init(|| Mutex::new(ForegroundTarget::None))
}
fn last_applied_hash() -> &'static Mutex<Option<(String, Instant)>> {
    LAST_APPLIED_HASH.get_or_init(|| Mutex::new(None))
}

pub fn validate_shortcut(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 80 || value.chars().any(char::is_control) {
        return Err("단축키를 확인해 주세요.".into());
    }
    let parts = value.split('+').map(str::trim).collect::<Vec<_>>();
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
        return Err("보조 키와 일반 키를 함께 입력해 주세요.".into());
    }
    let modifiers = [
        "Ctrl", "Control", "Shift", "Alt", "Option", "Cmd", "Command", "Super", "Meta",
    ];
    if !parts[..parts.len() - 1].iter().all(|part| {
        modifiers
            .iter()
            .any(|modifier| modifier.eq_ignore_ascii_case(part))
    }) || modifiers
        .iter()
        .any(|modifier| modifier.eq_ignore_ascii_case(parts[parts.len() - 1]))
    {
        return Err("지원하는 단축키 조합을 입력해 주세요.".into());
    }
    Ok(())
}

pub fn apply_shortcut(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    let manager = app.global_shortcut();
    manager
        .unregister_all()
        .map_err(|error| error.to_string())?;
    if settings.enabled && !super::session_token()?.is_empty() {
        validate_shortcut(&settings.shortcut)?;
        manager
            .register(settings.shortcut.as_str())
            .map_err(|_| "이 단축키는 다른 앱에서 사용 중입니다.".to_string())?;
    }
    Ok(())
}

fn root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_local_data_dir()
        .map_err(|error| error.to_string())?
        .join("clipboard");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

fn state_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(root(app)?.join("history.enc"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_state(app: &tauri::AppHandle) -> Result<LocalState, String> {
    let path = state_path(app)?;
    if !path.exists() {
        return Ok(LocalState::default());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() < 13 || bytes.len() > 40 * 1024 * 1024 {
        return Err("로컬 클립보드 기록이 올바르지 않습니다.".into());
    }
    let cipher = Aes256Gcm::new_from_slice(&super::history_key(false)?)
        .map_err(|_| "암호화 키가 올바르지 않습니다.".to_string())?;
    let plain = cipher
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .map_err(|_| "로컬 클립보드 기록을 열 수 없습니다.".to_string())?;
    serde_json::from_slice(&plain)
        .map_err(|_| "로컬 클립보드 기록이 올바르지 않습니다.".to_string())
}

fn save_state(app: &tauri::AppHandle, state: &LocalState) -> Result<(), String> {
    let plain = serde_json::to_vec(state).map_err(|error| error.to_string())?;
    let mut nonce = [0_u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| "암호화 nonce를 만들 수 없습니다.".to_string())?;
    let cipher = Aes256Gcm::new_from_slice(&super::history_key(true)?)
        .map_err(|_| "암호화 키가 올바르지 않습니다.".to_string())?;
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_slice())
        .map_err(|_| "클립보드 기록을 암호화할 수 없습니다.".to_string())?;
    let mut bytes = nonce.to_vec();
    bytes.extend(encrypted);
    super::atomic_write(&state_path(app)?, &bytes)
}

pub fn purge_local(app: &tauri::AppHandle) -> Result<(), String> {
    hide_popup(app);
    match fs::remove_file(state_path(app)?) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    super::delete_secret(super::HISTORY_KEY)
}

fn random_event_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "이벤트 ID를 만들 수 없습니다.".to_string())?;
    Ok(format!("clip_{}", hex(&bytes)))
}

fn publish(
    client: &reqwest::blocking::Client,
    settings: &Settings,
    token: &str,
    pending: &PendingItem,
) -> Result<ClipboardItem, String> {
    let response=client.post(super::endpoint(&settings.server_url,"/items")?).bearer_auth(token).json(&json!({
        "client_event_id":pending.client_event_id,"space_id":"personal","kind":pending.kind,"text":pending.text,
    })).send().map_err(|error|error.without_url().to_string())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("로그인이 만료되었습니다.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "클립보드 전송에 실패했습니다. ({})",
            response.status()
        ));
    }
    response
        .json()
        .map_err(|_| "클립보드 전송 응답이 올바르지 않습니다.".into())
}

fn sync_once(
    app: &tauri::AppHandle,
    client: &reqwest::blocking::Client,
    settings: &Settings,
    token: &str,
) -> Result<(), String> {
    let spaces = client
        .get(super::endpoint(&settings.server_url, "/spaces")?)
        .bearer_auth(token)
        .send()
        .map_err(|error| error.without_url().to_string())?;
    if spaces.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("로그인이 만료되었습니다.".into());
    }
    if !spaces.status().is_success() {
        return Err(format!(
            "동기화 서버가 응답하지 않았습니다. ({})",
            spaces.status()
        ));
    }
    let mut state = load_state(app).unwrap_or_default();
    let mut pending = Vec::new();
    for item in &state.pending {
        match publish(client, settings, token, item) {
            Ok(value) => {
                state
                    .items
                    .retain(|existing| existing.id != item.local_id && existing.id != value.id);
                state.items.insert(0, value)
            }
            Err(_) => pending.push(item.clone()),
        }
    }
    state.pending = pending;
    loop {
        let mut url = super::endpoint(&settings.server_url, "/feed")?;
        url.query_pairs_mut()
            .append_pair("after", &state.cursor.to_string())
            .append_pair("limit", "100");
        let response = client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|error| error.without_url().to_string())?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("로그인이 만료되었습니다.".into());
        }
        if !response.status().is_success() {
            return Err(format!(
                "클립보드 동기화에 실패했습니다. ({})",
                response.status()
            ));
        }
        let feed: Feed = response
            .json()
            .map_err(|_| "클립보드 동기화 응답이 올바르지 않습니다.".to_string())?;
        for event in feed.events {
            let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
            let id = event
                .get(if kind == "deleted" { "item_id" } else { "id" })
                .and_then(Value::as_str)
                .unwrap_or("");
            if kind == "deleted" {
                state.items.retain(|item| item.id != id)
            } else if kind == "published" {
                let item: ClipboardItem = serde_json::from_value(event)
                    .map_err(|_| "클립보드 항목이 올바르지 않습니다.".to_string())?;
                state.items.retain(|existing| existing.id != item.id);
                state.items.insert(0, item);
            }
        }
        state.cursor = feed.next_cursor;
        if !feed.has_more {
            break;
        }
    }
    state.items.truncate(MAX_ITEMS);
    save_state(app, &state)?;
    let _ = app.emit("clipboard-updated", ());
    Ok(())
}

pub fn sync_now(app: &tauri::AppHandle) -> Result<(), String> {
    let _guard = SYNC_LOCK
        .get_or_init(|| Mutex::new(()))
        .try_lock()
        .map_err(|_| "이미 동기화 중입니다.".to_string())?;
    let settings = super::read_settings(app)?;
    let token = super::session_token()?;
    if token.is_empty() {
        return Err("로그인이 필요합니다.".into());
    }
    let result = sync_once(app, &super::http_client()?, &settings, &token);
    let _ = app.emit(
        "sync-status",
        if result.is_ok() {
            "동기화됨"
        } else {
            "연결 확인 필요"
        },
    );
    result
}

fn classify(text: &str) -> &'static str {
    if (text.starts_with("https://") || text.starts_with("http://"))
        && !text.chars().any(char::is_whitespace)
    {
        "url"
    } else {
        "text"
    }
}

fn capture_local(
    app: &tauri::AppHandle,
    client: &reqwest::blocking::Client,
    settings: &Settings,
    token: &str,
    last_hash: &mut String,
) -> Result<(), String> {
    let Some(text) = read_clipboard()? else {
        return Ok(());
    };
    if text.is_empty() || text.len() > MAX_TEXT_BYTES || text.contains('\0') {
        return Ok(());
    }
    let hash = hex(&Sha256::digest(text.as_bytes()));
    if hash == *last_hash {
        return Ok(());
    }
    *last_hash = hash.clone();
    if last_applied_hash()
        .lock()
        .ok()
        .and_then(|value| value.clone())
        .is_some_and(|(applied, at)| applied == hash && at.elapsed() < Duration::from_secs(3))
    {
        return Ok(());
    }
    let client_event_id = random_event_id()?;
    let pending = PendingItem {
        local_id: format!("local_{client_event_id}"),
        client_event_id,
        kind: classify(&text).into(),
        text,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let mut state = load_state(app).unwrap_or_default();
    match publish(client, settings, token, &pending) {
        Ok(value) => {
            state.items.retain(|item| item.id != value.id);
            state.items.insert(0, value)
        }
        Err(_) => {
            state.items.insert(
                0,
                ClipboardItem {
                    seq: 0,
                    id: pending.local_id.clone(),
                    space_id: "personal".into(),
                    origin_device_id: settings.device_id.clone(),
                    kind: pending.kind.clone(),
                    text: pending.text.clone(),
                    created_at: pending.created_at.clone(),
                },
            );
            state.pending.push(pending)
        }
    }
    state.items.truncate(MAX_ITEMS);
    save_state(app, &state)?;
    let _ = app.emit("clipboard-updated", ());
    Ok(())
}

pub fn start_monitor(app: tauri::AppHandle) {
    thread::spawn(move || {
        let Ok(client) = super::http_client() else {
            return;
        };
        let mut last_sync = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .unwrap_or_else(Instant::now);
        let mut last_hash = String::new();
        let mut server_ready = false;
        loop {
            let settings = match super::read_settings(&app) {
                Ok(value) => value,
                Err(_) => {
                    thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };
            let token = super::session_token().unwrap_or_default();
            if settings.enabled && !token.is_empty() {
                if last_sync.elapsed() >= Duration::from_secs(3) {
                    let result = SYNC_LOCK
                        .get_or_init(|| Mutex::new(()))
                        .try_lock()
                        .ok()
                        .and_then(|_guard| sync_once(&app, &client, &settings, &token).ok());
                    server_ready = result.is_some();
                    let _ = app.emit(
                        "sync-status",
                        if server_ready {
                            "동기화됨"
                        } else {
                            "연결 확인 필요"
                        },
                    );
                    last_sync = Instant::now();
                }
                if server_ready {
                    let _ = capture_local(&app, &client, &settings, &token, &mut last_hash);
                }
                thread::sleep(Duration::from_millis(500));
            } else {
                server_ready = false;
                thread::sleep(Duration::from_secs(2));
            }
        }
    });
}

fn hide_popup(app: &tauri::AppHandle) {
    POPUP_ARMED.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("clipboard-popup") {
        let _ = window.hide();
    }
}
pub fn popup_focus_changed(app: &tauri::AppHandle, focused: bool) {
    let was_armed = POPUP_ARMED.swap(focused, Ordering::SeqCst);
    if was_armed && !focused {
        hide_popup(app)
    }
}
pub fn show_popup(app: &tauri::AppHandle) {
    if super::read_settings(app)
        .map(|settings| !settings.enabled)
        .unwrap_or(true)
    {
        return;
    }
    if let Ok(mut target) = foreground().lock() {
        *target = capture_foreground();
    }
    if let Some(window) = app.get_webview_window("clipboard-popup") {
        let _ = window.center();
        let _ = window.show();
        POPUP_ARMED.store(false, Ordering::SeqCst);
        if window.set_focus().is_ok() {
            POPUP_ARMED.store(true, Ordering::SeqCst);
        }
        let _ = window.emit("clipboard-open", ());
    }
}

#[tauri::command]
pub fn clipboard_dismiss(app: tauri::AppHandle) {
    hide_popup(&app)
}
#[tauri::command]
pub fn clipboard_history(app: tauri::AppHandle) -> Result<Vec<ClipboardItem>, String> {
    Ok(load_state(&app).unwrap_or_default().items)
}
#[tauri::command]
pub fn clipboard_select(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let item = load_state(&app)?
        .items
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "클립보드 항목을 찾을 수 없습니다.".to_string())?;
    write_clipboard(&item.text)?;
    let hash = hex(&Sha256::digest(item.text.as_bytes()));
    if let Ok(mut value) = last_applied_hash().lock() {
        *value = Some((hash, Instant::now()))
    }
    hide_popup(&app);
    thread::sleep(Duration::from_millis(100));
    paste_to_foreground();
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_clipboard() -> Result<Option<String>, String> {
    let output = Command::new("/usr/bin/pbpaste")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Ok(None);
    }
    String::from_utf8(output.stdout)
        .map(Some)
        .map_err(|_| "클립보드 텍스트가 UTF-8이 아닙니다.".into())
}
#[cfg(target_os = "macos")]
fn write_clipboard(text: &str) -> Result<(), String> {
    let mut child = Command::new("/usr/bin/pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "클립보드 입력을 열 수 없습니다.".to_string())?
        .write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    if child.wait().map_err(|error| error.to_string())?.success() {
        Ok(())
    } else {
        Err("클립보드에 쓸 수 없습니다.".into())
    }
}
#[cfg(target_os = "macos")]
fn capture_foreground() -> ForegroundTarget {
    let script="tell application \"System Events\" to get bundle identifier of first application process whose frontmost is true";
    Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|value| {
            !value.is_empty()
                && value.len() < 256
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        })
        .map(ForegroundTarget::Mac)
        .unwrap_or(ForegroundTarget::None)
}
#[cfg(target_os = "macos")]
fn paste_to_foreground() {
    let target = foreground()
        .lock()
        .ok()
        .map(|value| value.clone())
        .unwrap_or(ForegroundTarget::None);
    if let ForegroundTarget::Mac(bundle) = target {
        let script="on run argv\nset targetBundle to item 1 of argv\ntell application \"System Events\"\nset frontmost of first application process whose bundle identifier is targetBundle to true\ndelay 0.1\nkeystroke \"v\" using command down\nend tell\nend run";
        let _ = Command::new("/usr/bin/osascript")
            .args(["-e", script, "--", &bundle])
            .status();
    }
}

#[cfg(target_os = "windows")]
fn read_clipboard() -> Result<Option<String>, String> {
    use windows_sys::Win32::{
        System::Ole::CF_UNICODETEXT,
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
    };
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Ok(None);
        }
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if handle.is_null() {
            CloseClipboard();
            return Ok(None);
        }
        let size = GlobalSize(handle);
        if size == 0 || size > MAX_TEXT_BYTES * 2 + 2 {
            CloseClipboard();
            return Ok(None);
        }
        let pointer = GlobalLock(handle) as *const u16;
        if pointer.is_null() {
            CloseClipboard();
            return Ok(None);
        }
        let slice = std::slice::from_raw_parts(pointer, size / 2);
        let length = slice
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(slice.len());
        let text = String::from_utf16(&slice[..length])
            .map_err(|_| "클립보드 텍스트가 올바르지 않습니다.".to_string());
        GlobalUnlock(handle);
        CloseClipboard();
        text.map(Some)
    }
}
#[cfg(target_os = "windows")]
fn write_clipboard(text: &str) -> Result<(), String> {
    use windows_sys::Win32::{
        System::Ole::CF_UNICODETEXT,
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        },
    };
    let mut wide = text.encode_utf16().collect::<Vec<_>>();
    wide.push(0);
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("클립보드가 사용 중입니다.".into());
        }
        if EmptyClipboard() == 0 {
            CloseClipboard();
            return Err("클립보드를 비울 수 없습니다.".into());
        }
        let handle = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if handle.is_null() {
            CloseClipboard();
            return Err("클립보드 메모리를 만들 수 없습니다.".into());
        }
        let pointer = GlobalLock(handle) as *mut u16;
        if pointer.is_null() {
            CloseClipboard();
            return Err("클립보드를 잠글 수 없습니다.".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), pointer, wide.len());
        GlobalUnlock(handle);
        if SetClipboardData(CF_UNICODETEXT as u32, handle).is_null() {
            CloseClipboard();
            return Err("클립보드에 쓸 수 없습니다.".into());
        }
        CloseClipboard();
        Ok(())
    }
}
#[cfg(target_os = "windows")]
fn capture_foreground() -> ForegroundTarget {
    let hwnd = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if hwnd.is_null() {
        ForegroundTarget::None
    } else {
        ForegroundTarget::Windows(hwnd as isize)
    }
}
#[cfg(target_os = "windows")]
fn paste_to_foreground() {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL,
        },
        WindowsAndMessaging::SetForegroundWindow,
    };
    let target = foreground()
        .lock()
        .ok()
        .map(|value| value.clone())
        .unwrap_or(ForegroundTarget::None);
    if let ForegroundTarget::Windows(raw) = target {
        unsafe {
            SetForegroundWindow(raw as *mut _);
        }
        thread::sleep(Duration::from_millis(100));
        let key = |vk, flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [
            key(VK_CONTROL, 0),
            key(0x56, 0),
            key(0x56, KEYEVENTF_KEYUP),
            key(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn read_clipboard() -> Result<Option<String>, String> {
    Ok(None)
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn write_clipboard(_text: &str) -> Result<(), String> {
    Err("이 운영체제는 아직 지원하지 않습니다.".into())
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn capture_foreground() -> ForegroundTarget {
    ForegroundTarget::None
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn paste_to_foreground() {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_validation() {
        assert!(validate_shortcut(DEFAULT_SHORTCUT).is_ok());
        assert!(validate_shortcut("Command+Shift+C").is_ok());
        assert!(validate_shortcut("V").is_err());
        assert!(validate_shortcut("Ctrl+Shift").is_err());
    }
}
