use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use arboard::{Clipboard, ImageData};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex, OnceLock,
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
#[cfg(target_os = "windows")]
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use super::Settings;

#[cfg(test)]
const DEFAULT_SHORTCUT: &str = "Ctrl+Shift+V";
const MAX_ITEMS: usize = 500;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
const MAX_IMAGE_PIXELS: usize = 50_000_000;
const FILE_WAIT_SECONDS: u64 = 45;

#[cfg(target_os = "windows")]
static POPUP_CREATING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub seq: i64,
    pub id: String,
    pub space_id: String,
    pub origin_device_id: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub thumbnail_available: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PendingItem {
    client_event_id: String,
    kind: String,
    text: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    size_bytes: Option<u64>,
    #[serde(default)]
    thumbnail_base64: Option<String>,
    #[serde(default)]
    thumbnail_mime_type: Option<String>,
    local_id: String,
    created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LocalSource {
    item_id: String,
    path: PathBuf,
    size_bytes: u64,
    sha256: String,
    #[serde(default)]
    source_identity: Option<String>,
    #[serde(default)]
    managed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
struct LocalState {
    cursor: i64,
    #[serde(default)]
    items: Vec<ClipboardItem>,
    #[serde(default)]
    pending: Vec<PendingItem>,
    #[serde(default)]
    sources: Vec<LocalSource>,
}

#[derive(Debug, Deserialize)]
struct Feed {
    events: Vec<Value>,
    next_cursor: i64,
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct PendingFileRequests {
    requests: Vec<PendingFileRequest>,
}

#[derive(Debug, Deserialize)]
struct PendingFileRequest {
    #[serde(alias = "id")]
    request_id: String,
    item_id: String,
    size_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct FileRequestCreated {
    request_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct FileRequestStatus {
    status: String,
    #[serde(default)]
    content_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileUploadResult {
    content_sha256: String,
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
static SELECTION_STATE: OnceLock<SelectionState> = OnceLock::new();
#[cfg(target_os = "windows")]
static WINDOWS_STATE_CACHE: OnceLock<Mutex<Option<LocalState>>> = OnceLock::new();
#[cfg(target_os = "windows")]
static MONITOR_WAKE: OnceLock<(Mutex<MonitorWake>, Condvar)> = OnceLock::new();

#[cfg(target_os = "windows")]
#[derive(Default)]
struct MonitorWake {
    capture: bool,
    sync: bool,
}

#[derive(Default)]
struct SelectionState {
    inner: Mutex<SelectionStatus>,
}

#[derive(Default)]
struct SelectionStatus {
    popup_open: bool,
    active: bool,
    cancelled: bool,
    generation: u64,
}

struct SelectionGuard<'a> {
    state: &'a SelectionState,
    generation: u64,
}

impl SelectionState {
    fn open_popup(&self) {
        if let Ok(mut status) = self.inner.lock() {
            status.popup_open = true;
        }
    }

    fn close_popup(&self) {
        if let Ok(mut status) = self.inner.lock() {
            status.popup_open = false;
            if status.active {
                status.cancelled = true;
            }
        }
    }

    fn begin(&self) -> Result<SelectionGuard<'_>, String> {
        let mut status = self
            .inner
            .lock()
            .map_err(|_| "붙여넣기 상태를 확인할 수 없습니다.".to_string())?;
        if !status.popup_open {
            return Err("클립보드 기록을 다시 열고 선택해 주세요.".into());
        }
        if status.active {
            return Err("이미 항목을 붙여넣는 중입니다.".into());
        }
        status.active = true;
        status.cancelled = false;
        status.generation = status.generation.wrapping_add(1);
        Ok(SelectionGuard {
            state: self,
            generation: status.generation,
        })
    }
}

impl SelectionGuard<'_> {
    fn ensure_not_cancelled(&self) -> Result<(), String> {
        let status = self
            .state
            .inner
            .lock()
            .map_err(|_| "붙여넣기 상태를 확인할 수 없습니다.".to_string())?;
        if status.generation != self.generation || status.cancelled || !status.popup_open {
            Err("파일 받기를 취소했습니다.".into())
        } else {
            Ok(())
        }
    }
}

impl Drop for SelectionGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut status) = self.state.inner.lock() {
            if status.generation == self.generation {
                status.active = false;
                status.cancelled = false;
            }
        }
    }
}

fn selection_state() -> &'static SelectionState {
    SELECTION_STATE.get_or_init(SelectionState::default)
}

fn foreground() -> &'static Mutex<ForegroundTarget> {
    FOREGROUND.get_or_init(|| Mutex::new(ForegroundTarget::None))
}
fn last_applied_hash() -> &'static Mutex<Option<(String, Instant)>> {
    LAST_APPLIED_HASH.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "windows")]
fn request_monitor(capture: bool, sync: bool) {
    let (state, wake) =
        MONITOR_WAKE.get_or_init(|| (Mutex::new(MonitorWake::default()), Condvar::new()));
    if let Ok(mut state) = state.lock() {
        state.capture |= capture;
        state.sync |= sync;
        wake.notify_one();
    }
}

#[cfg(target_os = "macos")]
fn clipboard_change_marker() -> Option<u64> {
    let marker = objc2_app_kit::NSPasteboard::generalPasteboard().changeCount();
    (marker >= 0).then_some(marker as u64)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn clipboard_change_marker() -> Option<u64> {
    None
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

fn managed_sources_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let path = root(app)?.join("sources");
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn downloads_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let path = root(app)?.join("downloads");
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_state_from_disk(app: &tauri::AppHandle) -> Result<LocalState, String> {
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

#[cfg(target_os = "windows")]
fn load_state(app: &tauri::AppHandle) -> Result<LocalState, String> {
    let cache = WINDOWS_STATE_CACHE.get_or_init(|| Mutex::new(None));
    let mut cache = cache
        .lock()
        .map_err(|_| "로컬 클립보드 기록 상태를 읽을 수 없습니다.".to_string())?;
    if let Some(state) = cache.as_ref() {
        return Ok(state.clone());
    }
    let state = load_state_from_disk(app)?;
    *cache = Some(state.clone());
    Ok(state)
}

#[cfg(not(target_os = "windows"))]
fn load_state(app: &tauri::AppHandle) -> Result<LocalState, String> {
    load_state_from_disk(app)
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
    super::atomic_write(&state_path(app)?, &bytes)?;
    #[cfg(target_os = "windows")]
    if let Ok(mut cache) = WINDOWS_STATE_CACHE.get_or_init(|| Mutex::new(None)).lock() {
        *cache = Some(state.clone());
    }
    Ok(())
}

pub fn purge_local(app: &tauri::AppHandle) -> Result<(), String> {
    hide_popup(app);
    #[cfg(target_os = "windows")]
    {
        if let Ok(mut cache) = WINDOWS_STATE_CACHE.get_or_init(|| Mutex::new(None)).lock() {
            *cache = None;
        }
    }
    let key_result = super::delete_secret(super::HISTORY_KEY);
    let directory_result = match fs::remove_dir_all(root(app)?) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return key_result.and(Err(error.to_string())),
    };
    key_result.and(Ok(directory_result))
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
    let mut body = json!({
        "client_event_id":pending.client_event_id,"space_id":"personal","kind":pending.kind,"text":pending.text,
    });
    let object = body
        .as_object_mut()
        .expect("clipboard item body is an object");
    if let Some(value) = &pending.filename {
        object.insert("filename".into(), json!(value));
    }
    if let Some(value) = &pending.mime_type {
        object.insert("mime_type".into(), json!(value));
    }
    if let Some(value) = pending.size_bytes {
        object.insert("size_bytes".into(), json!(value));
    }
    if let Some(value) = &pending.thumbnail_base64 {
        object.insert("thumbnail_base64".into(), json!(value));
    }
    if let Some(value) = &pending.thumbnail_mime_type {
        object.insert("thumbnail_mime_type".into(), json!(value));
    }
    let response = client
        .post(super::endpoint(&settings.server_url, "/items")?)
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|error| error.without_url().to_string())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(super::AUTH_REQUIRED.into());
    }
    let status = response.status();
    if !status.is_success() {
        let code = response
            .json::<super::ApiError>()
            .ok()
            .map(|body| body.error);
        return match code.as_deref() {
            Some("quota_exceeded") => Err(
                "저장 용량 50MB를 모두 사용했습니다. 마이메모 웹에서 파일이나 기록을 정리해 주세요."
                    .into(),
            ),
            Some("clipboard_item_too_large") => {
                Err("이 클립보드 항목은 공유하기에 너무 큽니다.".into())
            }
            _ => Err(format!("클립보드 전송에 실패했습니다. ({status})")),
        };
    }
    response
        .json()
        .map_err(|_| "클립보드 전송 응답이 올바르지 않습니다.".into())
}

fn source_bytes(source: &LocalSource, expected_size: u64) -> Result<Vec<u8>, String> {
    let metadata =
        fs::metadata(&source.path).map_err(|_| "원본 파일을 찾을 수 없습니다.".to_string())?;
    if !metadata.is_file()
        || metadata.len() != source.size_bytes
        || metadata.len() != expected_size
        || metadata.len() == 0
        || metadata.len() > MAX_FILE_BYTES as u64
    {
        return Err("원본 파일이 복사한 뒤 변경되었거나 크기 제한을 넘었습니다.".into());
    }
    #[cfg(target_os = "windows")]
    if source.sha256.starts_with("pinned:") {
        let expected_identity = source
            .source_identity
            .as_ref()
            .ok_or_else(|| "지연 전송 파일 정보가 없습니다.".to_string())?;
        let before = windows_source_identity(&source.path, &metadata);
        if &before != expected_identity
            || source.sha256 != format!("pinned:{}", hex(&Sha256::digest(before.as_bytes())))
        {
            return Err("원본 파일이 복사한 뒤 변경되었습니다.".into());
        }
        let bytes =
            fs::read(&source.path).map_err(|_| "원본 파일을 읽을 수 없습니다.".to_string())?;
        let after_metadata = fs::metadata(&source.path)
            .map_err(|_| "원본 파일을 읽는 동안 변경되었습니다.".to_string())?;
        if windows_source_identity(&source.path, &after_metadata) != before
            || bytes.len() as u64 != expected_size
        {
            return Err("원본 파일을 읽는 동안 변경되었습니다.".into());
        }
        return Ok(bytes);
    }
    let bytes = fs::read(&source.path).map_err(|_| "원본 파일을 읽을 수 없습니다.".to_string())?;
    if hex(&Sha256::digest(&bytes)) != source.sha256 {
        return Err("원본 파일이 복사한 뒤 변경되었습니다.".into());
    }
    Ok(bytes)
}

fn fulfill_file_requests(
    client: &reqwest::blocking::Client,
    settings: &Settings,
    token: &str,
    state: &LocalState,
) -> Result<(), String> {
    let response = client
        .get(super::endpoint(
            &settings.server_url,
            "/file-requests/pending",
        )?)
        .bearer_auth(token)
        .send()
        .map_err(|error| error.without_url().to_string())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(super::AUTH_REQUIRED.into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "파일 요청을 확인하지 못했습니다. ({})",
            response.status()
        ));
    }
    let pending: PendingFileRequests = response
        .json()
        .map_err(|_| "파일 요청 응답이 올바르지 않습니다.".to_string())?;
    for request in pending.requests {
        let Some(source) = state
            .sources
            .iter()
            .find(|source| source.item_id == request.item_id)
        else {
            continue;
        };
        let Ok(bytes) = source_bytes(source, request.size_bytes) else {
            continue;
        };
        let content_sha256 = hex(&Sha256::digest(&bytes));
        let response = super::file_http_client()?
            .put(super::endpoint(
                &settings.server_url,
                &format!("/file-requests/{}/content", request.request_id),
            )?)
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .map_err(|error| error.without_url().to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "파일을 전송하지 못했습니다. ({})",
                response.status()
            ));
        }
        let uploaded: FileUploadResult = response
            .json()
            .map_err(|_| "파일 전송 응답이 올바르지 않습니다.".to_string())?;
        if uploaded.content_sha256 != content_sha256 {
            return Err("서버가 확인한 파일 내용이 원본과 다릅니다.".into());
        }
    }
    Ok(())
}

fn clean_local_sources(state: &mut LocalState) {
    let active = state
        .items
        .iter()
        .map(|item| item.id.clone())
        .chain(state.pending.iter().map(|item| item.local_id.clone()))
        .collect::<Vec<_>>();
    state.sources.retain(|source| {
        let keep = active.contains(&source.item_id) && source.path.is_file();
        if !keep && source.managed {
            let _ = fs::remove_file(&source.path);
        }
        keep
    });
}

fn sync_once(
    app: &tauri::AppHandle,
    client: &reqwest::blocking::Client,
    settings: &Settings,
    token: &str,
) -> Result<(), String> {
    let mut state = load_state(app).unwrap_or_default();
    let previous_state = state.clone();
    let mut pending = Vec::new();
    let mut pending_error = None;
    for item in state.pending.clone() {
        match publish(client, settings, token, &item) {
            Ok(value) => {
                state
                    .items
                    .retain(|existing| existing.id != item.local_id && existing.id != value.id);
                for source in &mut state.sources {
                    if source.item_id == item.local_id {
                        source.item_id = value.id.clone();
                    }
                }
                state.items.insert(0, value)
            }
            Err(error) => {
                if error == super::AUTH_REQUIRED {
                    return Err(error);
                }
                pending_error.get_or_insert(error);
                pending.push(item);
            }
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
            return Err(super::AUTH_REQUIRED.into());
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
    clean_local_sources(&mut state);
    let file_request_error = fulfill_file_requests(client, settings, token, &state).err();
    if state != previous_state {
        save_state(app, &state)?;
        let _ = app.emit("clipboard-updated", ());
    }
    if let Some(error) = pending_error {
        return Err(error);
    }
    if let Some(error) = file_request_error {
        return Err(error);
    }
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

fn thumbnail_base64(image: &DynamicImage) -> Option<String> {
    if image.width() as usize * image.height() as usize > MAX_IMAGE_PIXELS {
        return None;
    }
    let thumbnail = image.thumbnail(256, 256).to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 72)
        .encode_image(&thumbnail)
        .ok()?;
    (bytes.len() <= 256 * 1024).then(|| BASE64.encode(bytes))
}

fn image_from_clipboard() -> Option<DynamicImage> {
    let image = Clipboard::new().ok()?.get_image().ok()?;
    if image.width == 0
        || image.height == 0
        || image.width.checked_mul(image.height)? > MAX_IMAGE_PIXELS
        || image.bytes.len() != image.width.checked_mul(image.height)?.checked_mul(4)?
    {
        return None;
    }
    RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    )
    .map(DynamicImage::ImageRgba8)
}

#[cfg(target_os = "windows")]
fn windows_source_identity(path: &Path, metadata: &fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let stable_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("{}:{}:{}", stable_path.display(), metadata.len(), modified)
}

#[cfg(target_os = "windows")]
fn file_details(
    path: &Path,
) -> Result<(String, String, u64, String, Option<String>, Option<String>), String> {
    let canonical =
        fs::canonicalize(path).map_err(|_| "복사한 파일을 찾을 수 없습니다.".to_string())?;
    let metadata =
        fs::metadata(&canonical).map_err(|_| "복사한 파일 정보를 읽을 수 없습니다.".to_string())?;
    if !metadata.is_file() {
        return Err("폴더 붙여넣기는 지원하지 않습니다.".into());
    }
    if metadata.len() == 0 || metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("파일은 10MB 이하만 공유할 수 있습니다.".into());
    }
    let filename = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "파일 이름을 읽을 수 없습니다.".to_string())?
        .to_string();
    let identity = windows_source_identity(&canonical, &metadata);
    let digest = format!("pinned:{}", hex(&Sha256::digest(identity.as_bytes())));
    let mime = mime_guess::from_path(&canonical)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    Ok((filename, mime, metadata.len(), digest, None, Some(identity)))
}

#[cfg(not(target_os = "windows"))]
fn file_details(
    path: &Path,
) -> Result<(String, String, u64, String, Option<String>, Option<String>), String> {
    let metadata = fs::metadata(path).map_err(|_| "복사한 파일을 찾을 수 없습니다.".to_string())?;
    if !metadata.is_file() {
        return Err("폴더 붙여넣기는 지원하지 않습니다.".into());
    }
    if metadata.len() == 0 || metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("파일은 10MB 이하만 공유할 수 있습니다.".into());
    }
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "파일 이름을 읽을 수 없습니다.".to_string())?
        .to_string();
    let bytes = fs::read(path).map_err(|_| "복사한 파일을 읽을 수 없습니다.".to_string())?;
    let hash = hex(&Sha256::digest(&bytes));
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    let thumbnail = if mime.starts_with("image/") {
        image::open(path)
            .ok()
            .and_then(|image| thumbnail_base64(&image))
    } else {
        None
    };
    Ok((filename, mime, metadata.len(), hash, thumbnail, None))
}

fn should_capture(hash: &str, last_hash: &mut String) -> bool {
    if hash == *last_hash {
        return false;
    }
    *last_hash = hash.to_string();
    !last_applied_hash()
        .lock()
        .ok()
        .and_then(|value| value.clone())
        .is_some_and(|(applied, at)| applied == hash && at.elapsed() < Duration::from_secs(3))
}

#[cfg(target_os = "windows")]
fn privacy_markers_allow_capture(
    exclude_present: bool,
    history_present: bool,
    history_value: Option<u32>,
    cloud_present: bool,
    cloud_value: Option<u32>,
) -> bool {
    !exclude_present
        && (!history_present || history_value == Some(1))
        && (!cloud_present || cloud_value == Some(1))
}

#[cfg(target_os = "windows")]
fn windows_clipboard_capture_allowed() -> Result<bool, String> {
    use windows_sys::Win32::System::{
        DataExchange::{
            CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            RegisterClipboardFormatW,
        },
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    };

    let register = |name: &str| {
        let wide = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
    };
    let exclude = register("ExcludeClipboardContentFromMonitorProcessing");
    let history = register("CanIncludeInClipboardHistory");
    let cloud = register("CanUploadToCloudClipboard");
    if exclude == 0 || history == 0 || cloud == 0 {
        return Err("Windows 클립보드의 개인정보 보호 표시를 확인할 수 없습니다.".into());
    }

    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Ok(false);
        }
        let present = |format| IsClipboardFormatAvailable(format) != 0;
        let read_dword = |format| {
            let handle = GetClipboardData(format);
            if handle.is_null() || GlobalSize(handle) < std::mem::size_of::<u32>() {
                return None;
            }
            let pointer = GlobalLock(handle) as *const u32;
            if pointer.is_null() {
                return None;
            }
            let value = std::ptr::read_unaligned(pointer);
            GlobalUnlock(handle);
            Some(value)
        };
        let exclude_present = present(exclude);
        let history_present = present(history);
        let cloud_present = present(cloud);
        let allowed = privacy_markers_allow_capture(
            exclude_present,
            history_present,
            history_present.then(|| read_dword(history)).flatten(),
            cloud_present,
            cloud_present.then(|| read_dword(cloud)).flatten(),
        );
        CloseClipboard();
        Ok(allowed)
    }
}

fn store_capture(
    app: &tauri::AppHandle,
    client: &reqwest::blocking::Client,
    settings: &Settings,
    token: &str,
    pending: PendingItem,
    source: Option<LocalSource>,
) -> Result<(), String> {
    let mut state = load_state(app).unwrap_or_default();
    match publish(client, settings, token, &pending) {
        Ok(value) => {
            state.items.retain(|item| item.id != value.id);
            if let Some(mut source) = source {
                source.item_id = value.id.clone();
                state.sources.push(source);
            }
            state.items.insert(0, value)
        }
        Err(error) if error == super::AUTH_REQUIRED => {
            super::clear_local_auth(app)?;
            return Err("로그인이 만료되었습니다. 다시 로그인해 주세요.".into());
        }
        Err(error) => {
            state.items.insert(
                0,
                ClipboardItem {
                    seq: 0,
                    id: pending.local_id.clone(),
                    space_id: "personal".into(),
                    origin_device_id: settings.device_id.clone(),
                    kind: pending.kind.clone(),
                    text: pending.text.clone(),
                    filename: pending.filename.clone(),
                    mime_type: pending.mime_type.clone(),
                    size_bytes: pending.size_bytes,
                    thumbnail_available: pending.thumbnail_base64.is_some(),
                    created_at: pending.created_at.clone(),
                },
            );
            if let Some(source) = source {
                state.sources.push(source);
            }
            state.pending.push(pending);
            let _ = app.emit("sync-status", error);
        }
    }
    state.items.truncate(MAX_ITEMS);
    clean_local_sources(&mut state);
    save_state(app, &state)?;
    let _ = app.emit("clipboard-updated", ());
    Ok(())
}

fn capture_local(
    app: &tauri::AppHandle,
    client: &reqwest::blocking::Client,
    settings: &Settings,
    token: &str,
    last_hash: &mut String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if !windows_clipboard_capture_allowed()? {
        return Ok(());
    }
    if let Some(path) = read_file_clipboard()? {
        let (filename, mime, size, digest, thumbnail, source_identity) = file_details(&path)?;
        let hash = format!("file:{digest}");
        if !should_capture(&hash, last_hash) {
            return Ok(());
        }
        let client_event_id = random_event_id()?;
        let local_id = format!("local_{client_event_id}");
        return store_capture(
            app,
            client,
            settings,
            token,
            PendingItem {
                local_id: local_id.clone(),
                client_event_id,
                kind: "file".into(),
                text: filename.clone(),
                filename: Some(filename),
                mime_type: Some(mime),
                size_bytes: Some(size),
                thumbnail_base64: thumbnail.clone(),
                thumbnail_mime_type: thumbnail.map(|_| "image/jpeg".into()),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
            Some(LocalSource {
                item_id: local_id,
                path,
                size_bytes: size,
                sha256: digest,
                source_identity,
                managed: false,
            }),
        );
    }
    if let Some(text) = read_clipboard()? {
        if !text.is_empty() && text.len() <= MAX_TEXT_BYTES && !text.contains('\0') {
            let hash = format!("text:{}", hex(&Sha256::digest(text.as_bytes())));
            if !should_capture(&hash, last_hash) {
                return Ok(());
            }
            let client_event_id = random_event_id()?;
            return store_capture(
                app,
                client,
                settings,
                token,
                PendingItem {
                    local_id: format!("local_{client_event_id}"),
                    client_event_id,
                    kind: classify(&text).into(),
                    text,
                    filename: None,
                    mime_type: None,
                    size_bytes: None,
                    thumbnail_base64: None,
                    thumbnail_mime_type: None,
                    created_at: chrono::Utc::now().to_rfc3339(),
                },
                None,
            );
        }
    }
    let Some(image) = image_from_clipboard() else {
        return Ok(());
    };
    let rgba = image.to_rgba8();
    let digest = hex(&Sha256::digest(rgba.as_raw()));
    let hash = format!("image:{digest}");
    if !should_capture(&hash, last_hash) {
        return Ok(());
    }
    let client_event_id = random_event_id()?;
    let local_id = format!("local_{client_event_id}");
    let filename = format!(
        "clipboard-image-{}.png",
        chrono::Utc::now().timestamp_millis()
    );
    let mut cursor = Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|_| "이미지를 준비할 수 없습니다.".to_string())?;
    let bytes = cursor.into_inner();
    if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
        return Err("이미지는 10MB 이하만 공유할 수 있습니다.".into());
    }
    let path = managed_sources_dir(app)?.join(&filename);
    super::atomic_write(&path, &bytes)?;
    let thumbnail = thumbnail_base64(&DynamicImage::ImageRgba8(rgba));
    let result = store_capture(
        app,
        client,
        settings,
        token,
        PendingItem {
            local_id: local_id.clone(),
            client_event_id,
            kind: "image".into(),
            text: filename.clone(),
            filename: Some(filename),
            mime_type: Some("image/png".into()),
            size_bytes: Some(bytes.len() as u64),
            thumbnail_base64: thumbnail.clone(),
            thumbnail_mime_type: thumbnail.map(|_| "image/jpeg".into()),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
        Some(LocalSource {
            item_id: local_id,
            path: path.clone(),
            size_bytes: bytes.len() as u64,
            sha256: hex(&Sha256::digest(&bytes)),
            source_identity: None,
            managed: true,
        }),
    );
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

#[cfg(target_os = "windows")]
fn start_windows_clipboard_listener() {
    thread::spawn(|| unsafe {
        use windows_sys::Win32::{
            System::DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener},
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetMessageW, HWND_MESSAGE, MSG, WM_CLIPBOARDUPDATE,
                WS_DISABLED,
            },
        };
        let class = "STATIC\0".encode_utf16().collect::<Vec<_>>();
        let title = "MyMemo Clipboard Listener\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_DISABLED,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if window.is_null() || AddClipboardFormatListener(window) == 0 {
            if !window.is_null() {
                DestroyWindow(window);
            }
            return;
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            if message.message == WM_CLIPBOARDUPDATE {
                request_monitor(true, false);
            }
        }
        RemoveClipboardFormatListener(window);
        DestroyWindow(window);
    });
}

#[cfg(target_os = "windows")]
pub fn start_monitor(app: tauri::AppHandle) {
    start_windows_clipboard_listener();
    thread::spawn(move || {
        let Ok(client) = super::http_client() else {
            return;
        };
        let mut last_hash = String::new();
        let mut file_source_active = false;
        loop {
            let (state, wake) =
                MONITOR_WAKE.get_or_init(|| (Mutex::new(MonitorWake::default()), Condvar::new()));
            let Ok(mut state) = state.lock() else {
                return;
            };
            let mut timed_out = false;
            while !state.capture && !state.sync {
                if file_source_active {
                    let Ok((next, timeout)) = wake.wait_timeout(state, Duration::from_secs(20))
                    else {
                        return;
                    };
                    state = next;
                    if timeout.timed_out() {
                        timed_out = true;
                        break;
                    }
                } else {
                    let Ok(next) = wake.wait(state) else {
                        return;
                    };
                    state = next;
                }
            }
            let capture = state.capture;
            let sync = state.sync;
            state.capture = false;
            state.sync = false;
            drop(state);
            let settings = match super::read_settings(&app) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let token = super::session_token().unwrap_or_default();
            if !settings.enabled || token.is_empty() {
                file_source_active = false;
                continue;
            }
            if capture {
                let result = capture_local(&app, &client, &settings, &token, &mut last_hash);
                if let Err(error) = result {
                    let _ = app.emit("sync-status", error);
                } else {
                    file_source_active = load_state(&app)
                        .unwrap_or_default()
                        .sources
                        .iter()
                        .any(|source| source.sha256.starts_with("pinned:"));
                }
            }
            if sync {
                let result = SYNC_LOCK
                    .get_or_init(|| Mutex::new(()))
                    .try_lock()
                    .ok()
                    .map(|_guard| sync_once(&app, &client, &settings, &token));
                if matches!(&result, Some(Err(error)) if error == super::AUTH_REQUIRED) {
                    if let Err(error) = super::clear_local_auth(&app) {
                        let _ = app.emit("sync-status", error);
                    }
                    file_source_active = false;
                }
            } else if timed_out && file_source_active {
                let state = load_state(&app).unwrap_or_default();
                if let Err(error) = fulfill_file_requests(&client, &settings, &token, &state) {
                    let _ = app.emit("sync-status", error);
                }
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn start_monitor(app: tauri::AppHandle) {
    thread::spawn(move || {
        let Ok(client) = super::http_client() else {
            return;
        };
        let mut last_sync = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .unwrap_or_else(Instant::now);
        let mut last_hash = String::new();
        let mut last_clipboard_marker = None;
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
                        .map(|_guard| sync_once(&app, &client, &settings, &token));
                    if matches!(&result, Some(Err(error)) if error == super::AUTH_REQUIRED) {
                        if let Err(error) = super::clear_local_auth(&app) {
                            let _ = app.emit("sync-status", error);
                        }
                    }
                    server_ready = matches!(result, Some(Ok(())));
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
                let marker = clipboard_change_marker();
                let clipboard_changed = marker.is_none() || marker != last_clipboard_marker;
                if server_ready && clipboard_changed {
                    last_clipboard_marker = marker;
                    if let Err(error) =
                        capture_local(&app, &client, &settings, &token, &mut last_hash)
                    {
                        let _ = app.emit("sync-status", error);
                    }
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
    selection_state().close_popup();
    POPUP_ARMED.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("clipboard-popup") {
        #[cfg(target_os = "windows")]
        let _ = window.destroy();
        #[cfg(not(target_os = "windows"))]
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
    #[cfg(target_os = "windows")]
    request_monitor(false, true);
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
        selection_state().open_popup();
        let _ = window.emit("clipboard-open", ());
        return;
    }
    #[cfg(target_os = "windows")]
    {
        if POPUP_CREATING.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = app.clone();
        thread::spawn(move || {
            let result = WebviewWindowBuilder::new(
                &app,
                "clipboard-popup",
                WebviewUrl::App("popup.html".into()),
            )
            .title("클립보드 기록")
            .inner_size(440.0, 420.0)
            .center()
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(true)
            .build();
            POPUP_CREATING.store(false, Ordering::SeqCst);
            if let Ok(window) = result {
                POPUP_ARMED.store(false, Ordering::SeqCst);
                if window.set_focus().is_ok() {
                    POPUP_ARMED.store(true, Ordering::SeqCst);
                }
                selection_state().open_popup();
                let _ = window.emit("clipboard-open", ());
            }
        });
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
pub fn clipboard_thumbnail(app: tauri::AppHandle, id: String) -> Result<Option<String>, String> {
    let state = load_state(&app).unwrap_or_default();
    if let Some(pending) = state.pending.iter().find(|item| item.local_id == id) {
        return Ok(pending.thumbnail_base64.as_ref().map(|value| {
            format!(
                "data:{};base64,{}",
                pending
                    .thumbnail_mime_type
                    .as_deref()
                    .unwrap_or("image/jpeg"),
                value
            )
        }));
    }
    let item = state
        .items
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "클립보드 항목을 찾을 수 없습니다.".to_string())?;
    if !item.thumbnail_available {
        return Ok(None);
    }
    let settings = super::read_settings(&app)?;
    let token = super::session_token()?;
    let response = super::http_client()?
        .get(super::endpoint(
            &settings.server_url,
            &format!("/items/{id}/thumbnail"),
        )?)
        .bearer_auth(token)
        .send()
        .map_err(|error| error.without_url().to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        super::clear_local_auth(&app)?;
        return Err("로그인이 만료되었습니다. 다시 로그인해 주세요.".into());
    }
    if !response.status().is_success() {
        return Err("썸네일을 불러오지 못했습니다.".into());
    }
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .filter(|value| matches!(*value, "image/jpeg" | "image/png"))
        .ok_or_else(|| "썸네일 형식이 올바르지 않습니다.".to_string())?
        .to_string();
    let bytes = response
        .bytes()
        .map_err(|_| "썸네일을 읽지 못했습니다.".to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("썸네일이 너무 큽니다.".into());
    }
    Ok(Some(format!("data:{mime};base64,{}", BASE64.encode(bytes))))
}

fn request_file(
    app: &tauri::AppHandle,
    item: &ClipboardItem,
    selection: &SelectionGuard<'_>,
) -> Result<Vec<u8>, String> {
    selection.ensure_not_cancelled()?;
    let settings = super::read_settings(app)?;
    let token = super::session_token()?;
    let client = super::file_http_client()?;
    let created = client
        .post(super::endpoint(
            &settings.server_url,
            &format!("/items/{}/file-requests", item.id),
        )?)
        .bearer_auth(&token)
        .send()
        .map_err(|error| error.without_url().to_string())?;
    if created.status() == reqwest::StatusCode::UNAUTHORIZED {
        super::clear_local_auth(app)?;
        return Err("로그인이 만료되었습니다. 다시 로그인해 주세요.".into());
    }
    if !created.status().is_success() {
        return Err(format!(
            "파일을 요청하지 못했습니다. ({})",
            created.status()
        ));
    }
    let created: FileRequestCreated = created
        .json()
        .map_err(|_| "파일 요청 응답이 올바르지 않습니다.".to_string())?;
    let started = Instant::now();
    let mut status = created.status;
    let expected_sha256 = loop {
        selection.ensure_not_cancelled()?;
        if status == "pending" {
            if started.elapsed() >= Duration::from_secs(FILE_WAIT_SECONDS) {
                break None;
            }
            thread::sleep(Duration::from_secs(1));
        }
        selection.ensure_not_cancelled()?;
        let response = client
            .get(super::endpoint(
                &settings.server_url,
                &format!("/file-requests/{}", created.request_id),
            )?)
            .bearer_auth(&token)
            .send()
            .map_err(|error| error.without_url().to_string())?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            super::clear_local_auth(app)?;
            return Err("로그인이 만료되었습니다. 다시 로그인해 주세요.".into());
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err("파일 요청 시간이 지났습니다.".into());
        }
        if !response.status().is_success() {
            return Err("파일 전송 상태를 확인하지 못했습니다.".into());
        }
        let file_status = response
            .json::<FileRequestStatus>()
            .map_err(|_| "파일 전송 상태가 올바르지 않습니다.".to_string())?;
        status = file_status.status;
        if matches!(status.as_str(), "ready" | "consumed") {
            break file_status.content_sha256;
        }
        if status != "pending" {
            break None;
        }
    };
    if !matches!(status.as_str(), "ready" | "consumed") {
        return Err(
            "원본 기기가 응답하지 않습니다. 원본 기기에서 앱이 실행 중인지 확인해 주세요.".into(),
        );
    }
    let expected_sha256 = expected_sha256
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "파일 전송 상태가 올바르지 않습니다.".to_string())?;
    selection.ensure_not_cancelled()?;
    let response = client
        .get(super::endpoint(
            &settings.server_url,
            &format!("/file-requests/{}/content", created.request_id),
        )?)
        .bearer_auth(&token)
        .send()
        .map_err(|error| error.without_url().to_string())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        super::clear_local_auth(app)?;
        return Err("로그인이 만료되었습니다. 다시 로그인해 주세요.".into());
    }
    if !response.status().is_success() {
        return Err("파일을 내려받지 못했습니다.".into());
    }
    let response_sha256 = response
        .headers()
        .get("x-content-sha256")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "파일 전송 상태가 올바르지 않습니다.".to_string())?;
    if response_sha256 != expected_sha256 {
        return Err("받은 파일 내용이 올바르지 않습니다.".into());
    }
    let bytes = response
        .bytes()
        .map_err(|_| "파일을 내려받지 못했습니다.".to_string())?
        .to_vec();
    if bytes.is_empty()
        || bytes.len() > MAX_FILE_BYTES
        || item
            .size_bytes
            .is_some_and(|size| size != bytes.len() as u64)
    {
        return Err("받은 파일 크기가 올바르지 않습니다.".into());
    }
    if hex(&Sha256::digest(&bytes)) != expected_sha256 {
        return Err("받은 파일 내용이 올바르지 않습니다.".into());
    }
    Ok(bytes)
}

fn write_image_clipboard(bytes: &[u8]) -> Result<String, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|_| "받은 이미지를 열 수 없습니다.".to_string())?
        .to_rgba8();
    let hash = format!("image:{}", hex(&Sha256::digest(image.as_raw())));
    Clipboard::new()
        .and_then(|mut clipboard| {
            clipboard.set_image(ImageData {
                width: image.width() as usize,
                height: image.height() as usize,
                bytes: Cow::Owned(image.into_raw()),
            })
        })
        .map_err(|_| "이미지를 클립보드에 넣지 못했습니다.".to_string())?;
    Ok(hash)
}

#[tauri::command]
pub fn clipboard_select(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let selection = selection_state().begin()?;
    let state = load_state(&app)?;
    let item = state
        .items
        .iter()
        .find(|item| item.id == id)
        .cloned()
        .ok_or_else(|| "클립보드 항목을 찾을 수 없습니다.".to_string())?;
    let hash = if matches!(item.kind.as_str(), "file" | "image") {
        let source = state
            .sources
            .iter()
            .find(|source| source.item_id == item.id);
        let bytes = if let Some(source) = source {
            source_bytes(source, item.size_bytes.unwrap_or(source.size_bytes))?
        } else {
            let settings = super::read_settings(&app)?;
            if item.origin_device_id == settings.device_id {
                return Err("원본 파일을 찾을 수 없습니다. 파일을 다시 복사해 주세요.".into());
            }
            request_file(&app, &item, &selection)?
        };
        selection.ensure_not_cancelled()?;
        if item.kind == "image" {
            write_image_clipboard(&bytes)?
        } else {
            let filename = item.filename.as_deref().unwrap_or(&item.text);
            let path = downloads_dir(&app)?.join(format!("{}-{filename}", item.id));
            super::atomic_write(&path, &bytes)?;
            selection.ensure_not_cancelled()?;
            write_file_clipboard(&path)?;
            format!("file:{}", hex(&Sha256::digest(&bytes)))
        }
    } else {
        selection.ensure_not_cancelled()?;
        write_clipboard(&item.text)?;
        format!("text:{}", hex(&Sha256::digest(item.text.as_bytes())))
    };
    if let Ok(mut value) = last_applied_hash().lock() {
        *value = Some((hash, Instant::now()))
    }
    hide_popup(&app);
    thread::sleep(Duration::from_millis(100));
    paste_to_foreground();
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_file_clipboard() -> Result<Option<PathBuf>, String> {
    let script = "ObjC.import('AppKit');function run(){const p=$.NSPasteboard.generalPasteboard;const v=p.readObjectsForClassesOptions([$.NSURL],{NSPasteboardURLReadingFileURLsOnlyKey:true});if(!v||Number(v.count)!==1)return '';return ObjC.unwrap(v.objectAtIndex(0).path)}";
    let output = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}
#[cfg(target_os = "macos")]
fn write_file_clipboard(path: &Path) -> Result<(), String> {
    let script = "ObjC.import('AppKit');function run(argv){const p=$.NSPasteboard.generalPasteboard;p.clearContents;return p.writeObjects([$.NSURL.fileURLWithPath($(argv[0]))])?'ok':'fail'}";
    let output = Command::new("/usr/bin/osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            script,
            "--",
            &path.to_string_lossy(),
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "ok" {
        Ok(())
    } else {
        Err("파일을 클립보드에 넣지 못했습니다.".into())
    }
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
fn read_file_clipboard() -> Result<Option<PathBuf>, String> {
    use windows_sys::Win32::{
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
            Ole::CF_HDROP,
        },
        UI::Shell::DragQueryFileW,
    };
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Ok(None);
        }
        let handle = GetClipboardData(CF_HDROP as u32);
        if handle.is_null() {
            CloseClipboard();
            return Ok(None);
        }
        let count = DragQueryFileW(handle, u32::MAX, std::ptr::null_mut(), 0);
        if count != 1 {
            CloseClipboard();
            return Ok(None);
        }
        let length = DragQueryFileW(handle, 0, std::ptr::null_mut(), 0);
        let mut path = vec![0_u16; length as usize + 1];
        let copied = DragQueryFileW(handle, 0, path.as_mut_ptr(), path.len() as u32);
        CloseClipboard();
        if copied == 0 {
            return Ok(None);
        }
        path.truncate(copied as usize);
        Ok(Some(PathBuf::from(String::from_utf16_lossy(&path))))
    }
}
#[cfg(target_os = "windows")]
fn write_file_clipboard(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::POINT,
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            Ole::CF_HDROP,
        },
        UI::Shell::DROPFILES,
    };
    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.extend([0, 0]);
    let header_size = std::mem::size_of::<DROPFILES>();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("클립보드가 사용 중입니다.".into());
        }
        if EmptyClipboard() == 0 {
            CloseClipboard();
            return Err("클립보드를 비울 수 없습니다.".into());
        }
        let handle = GlobalAlloc(GMEM_MOVEABLE, header_size + wide.len() * 2);
        if handle.is_null() {
            CloseClipboard();
            return Err("클립보드 메모리를 만들 수 없습니다.".into());
        }
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            CloseClipboard();
            return Err("클립보드를 잠글 수 없습니다.".into());
        }
        std::ptr::write_unaligned(
            pointer as *mut DROPFILES,
            DROPFILES {
                pFiles: header_size as u32,
                pt: POINT { x: 0, y: 0 },
                fNC: 0,
                fWide: 1,
            },
        );
        std::ptr::copy_nonoverlapping(
            wide.as_ptr(),
            (pointer as *mut u8).add(header_size) as *mut u16,
            wide.len(),
        );
        GlobalUnlock(handle);
        if SetClipboardData(CF_HDROP as u32, handle).is_null() {
            CloseClipboard();
            return Err("파일을 클립보드에 넣지 못했습니다.".into());
        }
        CloseClipboard();
    }
    Ok(())
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
fn read_file_clipboard() -> Result<Option<PathBuf>, String> {
    Ok(None)
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn write_file_clipboard(_path: &Path) -> Result<(), String> {
    Err("이 운영체제는 아직 지원하지 않습니다.".into())
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

    #[test]
    fn pending_file_request_accepts_server_id_field() {
        let pending: PendingFileRequests = serde_json::from_str(
            r#"{"requests":[{"id":"request-1","item_id":"item-1","size_bytes":63}]}"#,
        )
        .expect("server pending response should deserialize");
        assert_eq!(pending.requests[0].request_id, "request-1");
    }

    #[test]
    fn selection_is_single_flight_and_popup_close_cancels_it() {
        let state = SelectionState::default();
        assert!(state.begin().is_err());

        state.open_popup();
        let selection = state.begin().expect("first selection should start");
        assert!(state.begin().is_err());
        assert!(selection.ensure_not_cancelled().is_ok());

        state.close_popup();
        assert_eq!(
            selection.ensure_not_cancelled().unwrap_err(),
            "파일 받기를 취소했습니다."
        );
        drop(selection);

        state.open_popup();
        assert!(state.begin().is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_privacy_markers_fail_closed() {
        assert!(privacy_markers_allow_capture(
            false, false, None, false, None
        ));
        assert!(!privacy_markers_allow_capture(
            true, false, None, false, None
        ));
        assert!(!privacy_markers_allow_capture(
            false,
            true,
            Some(0),
            false,
            None
        ));
        assert!(!privacy_markers_allow_capture(
            false, true, None, false, None
        ));
        assert!(!privacy_markers_allow_capture(
            false,
            false,
            None,
            true,
            Some(0)
        ));
        assert!(privacy_markers_allow_capture(
            false,
            true,
            Some(1),
            true,
            Some(1)
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn delayed_file_transfer_rejects_a_changed_source() {
        let path = std::env::temp_dir().join(format!(
            "mymemo-changed-source-{}.txt",
            random_event_id().expect("temporary id")
        ));
        fs::write(&path, b"before").expect("write original source");
        let metadata = fs::metadata(&path).expect("read original metadata");
        let identity = windows_source_identity(&path, &metadata);
        let source = LocalSource {
            item_id: "item-1".into(),
            path: path.clone(),
            size_bytes: metadata.len(),
            sha256: format!("pinned:{}", hex(&Sha256::digest(identity.as_bytes()))),
            source_identity: Some(identity),
            managed: false,
        };

        thread::sleep(Duration::from_millis(20));
        fs::write(&path, b"after!").expect("replace source bytes");
        let error = source_bytes(&source, source.size_bytes).expect_err("changed file must fail");
        assert!(error.contains("변경"));
        let _ = fs::remove_file(path);
    }
}
