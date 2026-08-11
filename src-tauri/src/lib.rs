mod clipboard;

use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
#[cfg(target_os = "windows")]
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

const DEFAULT_SERVER: &str = "https://memos.my";
const DEFAULT_HISTORY_LIMIT: usize = 200;
const MAX_HISTORY_LIMIT: usize = 500;
#[cfg(target_os = "macos")]
const DEFAULT_SHORTCUT: &str = "Command+Shift+V";
#[cfg(not(target_os = "macos"))]
const DEFAULT_SHORTCUT: &str = "Ctrl+Shift+V";
#[cfg(target_os = "windows")]
const KEYRING_SERVICE: &str = "my.memos.clipboard";
const SESSION_KEY: &str = "session-token";
const HISTORY_KEY: &str = "history-key";
pub(crate) const AUTH_REQUIRED: &str = "auth_required";

#[cfg(target_os = "windows")]
static MAIN_WINDOW_CREATING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub server_url: String,
    pub username: String,
    pub display_name: String,
    pub device_id: String,
    pub device_name: String,
    pub enabled: bool,
    pub shortcut: String,
    pub history_limit: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: DEFAULT_SERVER.into(),
            username: String::new(),
            display_name: String::new(),
            device_id: String::new(),
            device_name: default_device_name(),
            enabled: true,
            shortcut: DEFAULT_SHORTCUT.into(),
            history_limit: DEFAULT_HISTORY_LIMIT,
        }
    }
}

#[derive(Debug, Serialize)]
struct PublicSettings {
    server_url: String,
    username: String,
    display_name: String,
    device_name: String,
    logged_in: bool,
    enabled: bool,
    shortcut: String,
    history_limit: usize,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    token: String,
    user: LoginUser,
}

#[derive(Debug, Deserialize)]
struct LoginUser {
    username: String,
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: String,
}

static SESSION_CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static HISTORY_KEY_CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
#[cfg(target_os = "macos")]
static MACOS_SECRET_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub(crate) async fn run_blocking<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| "작업을 마치지 못했습니다. 다시 시도해 주세요.".to_string())?
}

fn secret_cache(account: &str) -> &'static Mutex<Option<String>> {
    if account == SESSION_KEY {
        SESSION_CACHE.get_or_init(|| Mutex::new(None))
    } else {
        HISTORY_KEY_CACHE.get_or_init(|| Mutex::new(None))
    }
}

#[cfg(target_os = "windows")]
fn keyring_entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, account)
        .map_err(|_| "운영체제 보안 저장소를 열 수 없습니다.".to_string())
}

#[cfg(target_os = "windows")]
fn load_platform_secret(account: &str) -> Result<String, String> {
    match keyring_entry(account)?.get_password() {
        Ok(value) => Ok(value),
        Err(keyring::Error::NoEntry) => Ok(String::new()),
        Err(_) => Err("운영체제 보안 저장소를 읽을 수 없습니다.".into()),
    }
}

#[cfg(target_os = "windows")]
fn store_platform_secret(account: &str, value: &str) -> Result<(), String> {
    keyring_entry(account)?
        .set_password(value)
        .map_err(|_| "운영체제 보안 저장소에 저장할 수 없습니다.".to_string())
}

#[cfg(target_os = "windows")]
fn delete_platform_secret(account: &str) -> Result<(), String> {
    match keyring_entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("운영체제 보안 저장소에서 삭제할 수 없습니다.".into()),
    }
}

#[cfg(target_os = "macos")]
fn secret_filename(account: &str) -> Result<&'static str, String> {
    match account {
        SESSION_KEY => Ok("session-token.secret"),
        HISTORY_KEY => Ok("history-key.secret"),
        _ => Err("로컬 보안 정보 이름이 올바르지 않습니다.".to_string()),
    }
}

fn valid_secret_value(account: &str, value: &str) -> bool {
    match account {
        SESSION_KEY => value.len() == 47 && value.starts_with("smc_"),
        HISTORY_KEY => value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn macos_secret_root() -> Result<&'static PathBuf, String> {
    MACOS_SECRET_ROOT
        .get()
        .ok_or_else(|| "로컬 보안 저장소가 준비되지 않았습니다.".to_string())
}

#[cfg(target_os = "macos")]
fn validate_private_macos_path(path: &Path, directory: bool) -> Result<fs::Metadata, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "로컬 보안 저장소를 확인할 수 없습니다.".to_string())?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err("로컬 보안 저장소 경로가 올바르지 않습니다.".to_string());
    }
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err("로컬 보안 저장소 권한이 안전하지 않습니다.".to_string());
    }
    Ok(metadata)
}

#[cfg(target_os = "macos")]
fn init_secret_store(app: &tauri::AppHandle) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let root = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("secrets");
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err("로컬 보안 저장소 경로가 올바르지 않습니다.".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(&root)
            .map_err(|_| "로컬 보안 저장소를 만들 수 없습니다.".to_string())?,
        Err(_) => return Err("로컬 보안 저장소를 확인할 수 없습니다.".to_string()),
    }
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .map_err(|_| "로컬 보안 저장소 권한을 설정할 수 없습니다.".to_string())?;
    validate_private_macos_path(&root, true)?;
    match MACOS_SECRET_ROOT.set(root.clone()) {
        Ok(()) => Ok(()),
        Err(_) if MACOS_SECRET_ROOT.get() == Some(&root) => Ok(()),
        Err(_) => Err("로컬 보안 저장소 경로가 바뀌었습니다.".to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
fn init_secret_store(_app: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn load_platform_secret(account: &str) -> Result<String, String> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };

    let path = macos_secret_root()?.join(secret_filename(account)?);
    let path_metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(_) => return Err("로컬 보안 정보를 확인할 수 없습니다.".to_string()),
    };
    validate_private_macos_path(&path, false)?;
    if path_metadata.len() > 128 {
        return Err("로컬 보안 정보가 올바르지 않습니다.".to_string());
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|_| "로컬 보안 정보를 읽을 수 없습니다.".to_string())?;
    let opened = file
        .metadata()
        .map_err(|_| "로컬 보안 정보를 확인할 수 없습니다.".to_string())?;
    if path_metadata.dev() != opened.dev() || path_metadata.ino() != opened.ino() {
        return Err("로컬 보안 정보가 읽는 동안 바뀌었습니다.".to_string());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(129)
        .read_to_end(&mut bytes)
        .map_err(|_| "로컬 보안 정보를 읽을 수 없습니다.".to_string())?;
    let value =
        String::from_utf8(bytes).map_err(|_| "로컬 보안 정보가 올바르지 않습니다.".to_string())?;
    if !valid_secret_value(account, &value) {
        return Err("로컬 보안 정보가 올바르지 않습니다.".to_string());
    }
    Ok(value)
}

#[cfg(target_os = "macos")]
fn store_platform_secret(account: &str, value: &str) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    if !valid_secret_value(account, value) {
        return Err("로컬 보안 정보가 올바르지 않습니다.".to_string());
    }
    let root = macos_secret_root()?;
    validate_private_macos_path(root, true)?;
    let target = root.join(secret_filename(account)?);
    let mut suffix = [0_u8; 8];
    getrandom::fill(&mut suffix).map_err(|_| "임시 파일을 만들 수 없습니다.".to_string())?;
    let temp = root.join(format!(".secret-{}.tmp", hex(&suffix)));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)
            .map_err(|_| "로컬 보안 정보를 저장할 수 없습니다.".to_string())?;
        file.write_all(value.as_bytes())
            .map_err(|_| "로컬 보안 정보를 저장할 수 없습니다.".to_string())?;
        file.sync_all()
            .map_err(|_| "로컬 보안 정보를 저장할 수 없습니다.".to_string())?;
        drop(file);
        fs::rename(&temp, &target)
            .map_err(|_| "로컬 보안 정보를 저장할 수 없습니다.".to_string())?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
            .map_err(|_| "로컬 보안 저장소 권한을 설정할 수 없습니다.".to_string())?;
        validate_private_macos_path(&target, false)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(target_os = "macos")]
fn delete_platform_secret(account: &str) -> Result<(), String> {
    let path = macos_secret_root()?.join(secret_filename(account)?);
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            validate_private_macos_path(&path, false)?;
            fs::remove_file(path).map_err(|_| "로컬 보안 정보를 삭제할 수 없습니다.".to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("로컬 보안 정보를 확인할 수 없습니다.".to_string()),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn load_platform_secret(_account: &str) -> Result<String, String> {
    Ok(String::new())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn store_platform_secret(_account: &str, _value: &str) -> Result<(), String> {
    Err("이 운영체제에서는 보안 저장소를 지원하지 않습니다.".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn delete_platform_secret(_account: &str) -> Result<(), String> {
    Ok(())
}

fn set_secret(account: &str, value: &str) -> Result<(), String> {
    store_platform_secret(account, value)?;
    *secret_cache(account)
        .lock()
        .map_err(|_| "보안 저장소 상태를 읽을 수 없습니다.".to_string())? = Some(value.to_string());
    Ok(())
}

fn get_secret(account: &str) -> Result<String, String> {
    let mut cache = secret_cache(account)
        .lock()
        .map_err(|_| "보안 저장소 상태를 읽을 수 없습니다.".to_string())?;
    if cache.is_none() {
        *cache = Some(load_platform_secret(account)?);
    }
    Ok(cache.clone().unwrap_or_default())
}

fn delete_secret(account: &str) -> Result<(), String> {
    *secret_cache(account)
        .lock()
        .map_err(|_| "보안 저장소 상태를 읽을 수 없습니다.".to_string())? = Some(String::new());
    delete_platform_secret(account)
}

pub(crate) fn history_key(create: bool) -> Result<[u8; 32], String> {
    let mut value = get_secret(HISTORY_KEY)?;
    if value.is_empty() && create {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| "암호화 키를 만들 수 없습니다.".to_string())?;
        value = hex(&bytes);
        set_secret(HISTORY_KEY, &value)?;
    }
    let bytes =
        decode_hex(&value).ok_or_else(|| "클립보드 암호화 키가 올바르지 않습니다.".to_string())?;
    bytes
        .try_into()
        .map_err(|_| "클립보드 암호화 키가 올바르지 않습니다.".to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "내 기기".into())
        .chars()
        .take(80)
        .collect()
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root.join("settings.json"))
}

pub(crate) fn read_settings(app: &tauri::AppHandle) -> Result<Settings, String> {
    let path = settings_path(app)?;
    let mut settings = if path.exists() {
        serde_json::from_slice::<Settings>(&fs::read(&path).map_err(|error| error.to_string())?)
            .map_err(|_| "설정 파일이 올바르지 않습니다.".to_string())?
    } else {
        Settings::default()
    };
    if settings.device_id.is_empty() {
        let mut id = [0_u8; 16];
        getrandom::fill(&mut id).map_err(|_| "기기 ID를 만들 수 없습니다.".to_string())?;
        settings.device_id = format!("device_{}", hex(&id));
        write_settings(app, &settings)?;
    }
    Ok(settings)
}

fn write_settings(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app)?;
    atomic_write(
        &path,
        &serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?,
    )
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "저장 경로가 올바르지 않습니다.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut suffix = [0_u8; 8];
    getrandom::fill(&mut suffix).map_err(|_| "임시 파일을 만들 수 없습니다.".to_string())?;
    let temp = parent.join(format!(".clipboard-{}.tmp", hex(&suffix)));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        replace_file(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

pub(crate) fn session_token() -> Result<String, String> {
    get_secret(SESSION_KEY)
}

pub(crate) fn endpoint(server_url: &str, path: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(server_url)
        .map_err(|_| "서버 주소가 올바르지 않습니다.".to_string())?;
    let production = url.scheme() == "https"
        && url.host_str() == Some("memos.my")
        && url.port_or_known_default() == Some(443);
    let safe_local = cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if (!production && !safe_local)
        || url.username() != ""
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("마이메모 HTTPS 주소를 확인해 주세요.".into());
    }
    url.set_path(&format!("/api/clipboard/v1{path}"));
    url.set_query(None);
    Ok(url)
}

pub(crate) fn http_client() -> Result<reqwest::blocking::Client, String> {
    let builder = reqwest::blocking::Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent("mymemo-clipboard/0.2.5");
    #[cfg(target_os = "windows")]
    let builder = builder.pool_max_idle_per_host(0);
    builder
        .build()
        .map_err(|_| "네트워크를 준비할 수 없습니다.".to_string())
}

pub(crate) fn file_http_client() -> Result<reqwest::blocking::Client, String> {
    let builder = reqwest::blocking::Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(90))
        .user_agent("mymemo-clipboard/0.2.5");
    #[cfg(target_os = "windows")]
    let builder = builder.pool_max_idle_per_host(0);
    builder
        .build()
        .map_err(|_| "파일 전송을 준비할 수 없습니다.".to_string())
}

pub(crate) fn clear_local_auth(app: &tauri::AppHandle) -> Result<(), String> {
    let mut cleanup_failed = false;
    cleanup_failed |= delete_secret(SESSION_KEY).is_err();
    cleanup_failed |= clipboard::purge_local(app).is_err();
    if let Ok(mut settings) = read_settings(app) {
        settings.username.clear();
        settings.display_name.clear();
        cleanup_failed |= write_settings(app, &settings).is_err();
    } else {
        cleanup_failed = true;
    }
    cleanup_failed |= app.global_shortcut().unregister_all().is_err();
    let _ = app.emit("auth-required", ());
    if cleanup_failed {
        Err("로그아웃 정보 일부를 지우지 못했습니다. 앱을 종료한 뒤 다시 실행해 주세요.".into())
    } else {
        Ok(())
    }
}

fn load_settings_blocking(app: tauri::AppHandle) -> Result<PublicSettings, String> {
    let settings = read_settings(&app)?;
    let token = session_token()?;
    Ok(PublicSettings {
        server_url: settings.server_url,
        username: settings.username,
        display_name: settings.display_name,
        device_name: settings.device_name,
        logged_in: !token.is_empty(),
        enabled: settings.enabled,
        shortcut: settings.shortcut,
        history_limit: settings.history_limit,
    })
}

#[tauri::command]
async fn load_settings(app: tauri::AppHandle) -> Result<PublicSettings, String> {
    run_blocking(move || load_settings_blocking(app)).await
}

fn login_blocking(
    app: tauri::AppHandle,
    server_url: String,
    username: String,
    password: String,
) -> Result<(), String> {
    if password.is_empty() || password.len() > 256 {
        return Err("아이디 또는 비밀번호를 확인해 주세요.".into());
    }
    let mut settings = read_settings(&app)?;
    let url = endpoint(&server_url, "/login")?;
    let response = http_client()?.post(url).json(&json!({
        "username": username, "password": password, "device_id": settings.device_id, "device_name": settings.device_name,
    })).send().map_err(|error| error.without_url().to_string())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("아이디 또는 비밀번호를 확인해 주세요.".into());
    }
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("로그인 시도가 많습니다. 잠시 후 다시 시도해 주세요.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "로그인 서버가 응답하지 않았습니다. ({})",
            response.status()
        ));
    }
    let result: LoginResponse = response
        .json()
        .map_err(|_| "로그인 응답이 올바르지 않습니다.".to_string())?;
    if !result.token.starts_with("smc_") || result.token.len() != 47 {
        return Err("로그인 응답이 올바르지 않습니다.".into());
    }
    if settings.username != result.user.username
        || settings.server_url.trim_end_matches('/') != server_url.trim_end_matches('/')
    {
        clipboard::purge_local(&app)?;
    }
    set_secret(SESSION_KEY, &result.token)?;
    settings.server_url = server_url.trim_end_matches('/').to_string();
    settings.username = result.user.username;
    settings.display_name = result.user.display_name;
    settings.enabled = true;
    write_settings(&app, &settings)?;
    clipboard::apply_shortcut(&app, &settings)?;
    let _ = app.emit("sync-status", "동기화 준비됨");
    Ok(())
}

#[tauri::command]
async fn login(
    app: tauri::AppHandle,
    server_url: String,
    username: String,
    password: String,
) -> Result<(), String> {
    run_blocking(move || login_blocking(app, server_url, username, password)).await
}

fn validate_current_password(password: &str) -> Result<(), String> {
    if password.is_empty() || password.encode_utf16().count() > 256 {
        return Err("현재 비밀번호를 입력해 주세요.".into());
    }
    Ok(())
}

fn validate_new_password(password: &str) -> Result<(), String> {
    let length = password.encode_utf16().count();
    if length < 8 || length > 256 || password.chars().any(char::is_control) {
        return Err("비밀번호는 제어 문자를 제외하고 8자 이상 입력해 주세요.".into());
    }
    Ok(())
}

fn change_password_blocking(
    app: tauri::AppHandle,
    current_password: String,
    new_password: String,
) -> Result<(), String> {
    validate_current_password(&current_password)?;
    validate_new_password(&new_password)?;
    if current_password == new_password {
        return Err("현재 비밀번호와 다른 비밀번호를 입력해 주세요.".into());
    }
    let settings = read_settings(&app)?;
    let token = session_token()?;
    if token.is_empty() {
        return Err("다시 로그인해 주세요.".into());
    }
    let response = http_client()?
        .post(endpoint(&settings.server_url, "/password")?)
        .bearer_auth(&token)
        .json(&json!({
            "current_password": current_password,
            "new_password": new_password,
        }))
        .send()
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let code = response.json::<ApiError>().ok().map(|body| body.error);
    match (status, code.as_deref()) {
        (_, Some("current_password_invalid" | "invalid_credentials")) => {
            Err("현재 비밀번호가 맞지 않습니다.".into())
        }
        (_, Some("password_unchanged")) => {
            Err("현재 비밀번호와 다른 비밀번호를 입력해 주세요.".into())
        }
        (_, Some("password_invalid")) => {
            Err("비밀번호는 제어 문자를 제외하고 8자 이상 입력해 주세요.".into())
        }
        (reqwest::StatusCode::UNAUTHORIZED, _) => {
            Err("로그인이 만료되었습니다. 다시 로그인해 주세요.".into())
        }
        (reqwest::StatusCode::TOO_MANY_REQUESTS, _) => {
            Err("요청이 많습니다. 잠시 후 다시 시도해 주세요.".into())
        }
        _ => Err(format!("비밀번호를 변경하지 못했습니다. ({status})")),
    }
}

#[tauri::command]
async fn change_password(
    app: tauri::AppHandle,
    current_password: String,
    new_password: String,
) -> Result<(), String> {
    run_blocking(move || change_password_blocking(app, current_password, new_password)).await
}

fn logout_blocking(app: tauri::AppHandle) -> Result<(), String> {
    let settings = read_settings(&app)?;
    let token = session_token().unwrap_or_default();
    if !token.is_empty() {
        if let Ok(client) = http_client() {
            if let Ok(url) = endpoint(&settings.server_url, "/logout") {
                let _ = client.post(url).bearer_auth(&token).send();
            }
        }
    }
    clear_local_auth(&app)
}

#[tauri::command]
async fn logout(app: tauri::AppHandle) -> Result<(), String> {
    run_blocking(move || logout_blocking(app)).await
}

fn save_preferences_blocking(
    app: tauri::AppHandle,
    enabled: bool,
    shortcut: String,
    history_limit: usize,
) -> Result<(), String> {
    clipboard::validate_shortcut(&shortcut)?;
    if !(1..=MAX_HISTORY_LIMIT).contains(&history_limit) {
        return Err("최근 기록 개수는 1개에서 500개 사이로 입력해 주세요.".into());
    }
    let previous = read_settings(&app)?;
    let mut next = previous.clone();
    next.enabled = enabled;
    next.shortcut = shortcut;
    next.history_limit = history_limit;
    if let Err(error) = clipboard::apply_shortcut(&app, &next) {
        let _ = clipboard::apply_shortcut(&app, &previous);
        return Err(error);
    }
    if let Err(error) = write_settings(&app, &next) {
        let _ = clipboard::apply_shortcut(&app, &previous);
        return Err(error);
    }
    if previous.enabled && !enabled {
        clipboard::purge_local(&app)?;
    } else {
        clipboard::trim_history(&app, history_limit)?;
    }
    Ok(())
}

#[tauri::command]
async fn save_preferences(
    app: tauri::AppHandle,
    enabled: bool,
    shortcut: String,
    history_limit: usize,
) -> Result<(), String> {
    run_blocking(move || save_preferences_blocking(app, enabled, shortcut, history_limit)).await
}

fn sync_now_blocking(app: tauri::AppHandle) -> Result<(), String> {
    match clipboard::sync_now(&app) {
        Err(error) if error == AUTH_REQUIRED => {
            clear_local_auth(&app)?;
            Err("로그인이 만료되었습니다. 다시 로그인해 주세요.".into())
        }
        result => result,
    }
}

#[tauri::command]
async fn sync_now(app: tauri::AppHandle) -> Result<(), String> {
    run_blocking(move || sync_now_blocking(app)).await
}

#[cfg(target_os = "windows")]
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    if MAIN_WINDOW_CREATING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let result = WebviewWindowBuilder::new(&app, "main", WebviewUrl::App("index.html".into()))
            .title("MyMemo Clipboard")
            .inner_size(820.0, 760.0)
            .min_inner_size(620.0, 600.0)
            .resizable(true)
            .build();
        MAIN_WINDOW_CREATING.store(false, std::sync::atomic::Ordering::SeqCst);
        if let Ok(window) = result {
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        clipboard::show_popup(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            load_settings,
            login,
            change_password,
            logout,
            save_preferences,
            sync_now,
            clipboard::clipboard_history,
            clipboard::clipboard_thumbnail,
            clipboard::clipboard_select,
            clipboard::clipboard_dismiss,
        ])
        .on_window_event(|window, event| {
            if window.label() == "clipboard-popup" {
                if let tauri::WindowEvent::Focused(focused) = event {
                    clipboard::popup_focus_changed(window.app_handle(), *focused);
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                #[cfg(target_os = "windows")]
                {
                    api.prevent_close();
                    let _ = window.destroy();
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .setup(|app| {
            init_secret_store(app.handle())?;
            let open = MenuItem::with_id(app, "open", "설정 열기", true, None::<&str>)?;
            let history = MenuItem::with_id(app, "history", "클립보드 기록", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&open, &history, &PredefinedMenuItem::separator(app)?, &quit],
            )?;
            TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("MyMemo Clipboard")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main_window(app),
                    "history" => clipboard::show_popup(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            let shortcut_app = app.handle().clone();
            std::thread::spawn(move || {
                if let Ok(settings) = read_settings(&shortcut_app) {
                    let _ = clipboard::apply_shortcut(&shortcut_app, &settings);
                }
            });
            clipboard::start_monitor(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build MyMemo Clipboard")
        .run(|_, _event| {
            #[cfg(target_os = "windows")]
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = _event
            {
                api.prevent_exit();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_endpoints_are_pinned_to_memos_my() {
        assert_eq!(
            endpoint("https://memos.my", "/feed").unwrap().as_str(),
            "https://memos.my/api/clipboard/v1/feed"
        );
        assert!(endpoint("https://attacker.example", "/login").is_err());
        assert!(endpoint("https://memos.my.attacker.example", "/login").is_err());
        assert!(endpoint("https://memos.my:8443", "/login").is_err());
        assert!(endpoint("https://memos.my/path", "/login").is_err());
        assert!(endpoint("https://user@memos.my", "/login").is_err());
    }

    #[test]
    fn password_change_accepts_existing_eight_character_passwords() {
        assert!(validate_current_password("12345678").is_ok());
        assert!(validate_new_password("12345678").is_ok());
        assert!(validate_current_password("").is_err());
        assert!(validate_new_password("1234567").is_err());
    }

    #[test]
    fn local_secret_values_use_exact_shapes() {
        assert!(valid_secret_value(
            SESSION_KEY,
            &format!("smc_{}", "A".repeat(43))
        ));
        assert!(!valid_secret_value(
            SESSION_KEY,
            &format!("legacy_{}", "z".repeat(40))
        ));
        assert!(valid_secret_value(HISTORY_KEY, &"a1".repeat(32)));
        assert!(!valid_secret_value(SESSION_KEY, "smc_short"));
        assert!(!valid_secret_value(HISTORY_KEY, &"g0".repeat(32)));
        assert!(!valid_secret_value("unknown", &"a".repeat(64)));
    }

    #[test]
    fn history_limit_defaults_to_two_hundred() {
        let settings = Settings::default();
        assert_eq!(settings.history_limit, 200);
        let restored: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(restored.history_limit, 200);
    }
}
