import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src-tauri/src/clipboard.rs', import.meta.url), 'utf8');
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
assert.match(windowsMonitor, /wake\.wait\(state\)/);
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

console.log('windows idle clipboard regression: ok');
