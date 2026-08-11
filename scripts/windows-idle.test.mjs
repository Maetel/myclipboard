import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const readSource = (path) => readFileSync(new URL(path, import.meta.url), 'utf8').replaceAll('\r\n', '\n');
const source = readSource('../src-tauri/src/clipboard.rs');
const appSource = readSource('../src-tauri/src/lib.rs');
const popupSource = readSource('../src/popup.ts');
const popupHtml = readSource('../popup.html');
const mainHtml = readSource('../index.html');
const windowsConfig = JSON.parse(
  readFileSync(new URL('../src-tauri/tauri.windows.conf.json', import.meta.url), 'utf8'),
);
const windowsMonitorStart = source.indexOf(
  '#[cfg(target_os = "windows")]\npub fn start_monitor(app: tauri::AppHandle)',
);
const nonWindowsMonitorStart = source.indexOf(
  '#[cfg(not(target_os = "windows"))]\npub fn start_monitor(app: tauri::AppHandle)',
  windowsMonitorStart,
);
assert.ok(windowsMonitorStart >= 0 && nonWindowsMonitorStart > windowsMonitorStart);
const windowsMonitor = source.slice(windowsMonitorStart, nonWindowsMonitorStart);
assert.match(source, /AddClipboardFormatListener/);
assert.match(source, /WM_CLIPBOARDUPDATE/);
assert.match(source, /ExcludeClipboardContentFromMonitorProcessing/);
assert.match(source, /CanIncludeInClipboardHistory/);
assert.match(source, /CanUploadToCloudClipboard/);
assert.match(source, /const SYNC_INTERVAL: Duration = Duration::from_secs\(2\)/);
assert.match(windowsMonitor, /wake\.wait_timeout\(state, SYNC_INTERVAL\)/);
assert.doesNotMatch(windowsMonitor, /from_millis\(500\)/);
assert.doesNotMatch(windowsMonitor, /from_secs\(3\)/);
assert.doesNotMatch(windowsMonitor, /GetClipboardSequenceNumber/);
assert.doesNotMatch(windowsMonitor, /Command::new|cmd\.exe|powershell\.exe/i);

const windowsDetailsStart = source.indexOf(
  '#[cfg(target_os = "windows")]\nfn file_details(',
);
const nonWindowsDetailsStart = source.indexOf(
  '#[cfg(not(target_os = "windows"))]\nfn file_details(',
  windowsDetailsStart,
);
assert.ok(windowsDetailsStart >= 0 && nonWindowsDetailsStart > windowsDetailsStart);
const windowsDetails = source.slice(windowsDetailsStart, nonWindowsDetailsStart);
assert.match(windowsDetails, /fs::canonicalize/);
assert.match(windowsDetails, /windows_source_identity/);
assert.doesNotMatch(windowsDetails, /fs::read|image::open|thumbnail_base64/);

assert.doesNotMatch(source, /"\/spaces"/);
assert.match(source, /headers\(\)[\s\S]*"x-content-sha256"/);
assert.match(source, /hex\(&Sha256::digest\(&bytes\)\) != expected_sha256/);
assert.match(source, /content_sha256 != content_sha256/);

assert.deepEqual(windowsConfig.app.windows, []);
assert.deepEqual(windowsConfig.bundle.targets, ['nsis']);
assert.match(appSource, /WebviewWindowBuilder::new\([\s\S]*"main"/);
assert.match(source, /WebviewWindowBuilder::new\([\s\S]*"clipboard-popup"/);
assert.match(appSource, /MAIN_WINDOW_CREATING\.swap/);
assert.match(appSource, /url\.host_str\(\) == Some\("memos\.my"\)/);
assert.doesNotMatch(mainHtml, /id="serverUrl"/);
assert.match(appSource, /ExitRequested[\s\S]*code: None[\s\S]*prevent_exit/);
assert.equal(appSource.match(/pool_max_idle_per_host\(0\)/g)?.length, 2);
assert.match(source, /POPUP_CREATING\.swap/);
assert.match(source, /struct SelectionState/);
assert.match(source, /이미 항목을 붙여넣는 중입니다/);
assert.match(source, /selection\.ensure_not_cancelled\(\)\?/);
assert.match(source, /selection_state\(\)\.close_popup\(\)/);
assert.ok(source.match(/StatusCode::UNAUTHORIZED[\s\S]{0,180}clear_local_auth/g)?.length >= 4);
assert.match(popupSource, /if \(selecting\) return/);
assert.match(popupSource, /Esc로 취소/);
assert.match(popupSource, /aria-activedescendant/);
assert.doesNotMatch(popupSource, /origin_device_id\.slice/);
assert.match(popupHtml, /aria-activedescendant|aria-controls="clipboardItems"/);
assert.match(appSource, /#\[cfg\(target_os = "windows"\)\][\s\S]*window\.destroy\(\)/);
assert.match(source, /#\[cfg\(target_os = "windows"\)\][\s\S]*window\.destroy\(\)/);

console.log('windows idle clipboard regression: ok');
